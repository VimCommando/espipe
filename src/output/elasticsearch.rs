mod bulk_response;

use super::{BulkAction, Sender};
use crate::output::OutputPreflightConfig;
use bulk_response::BulkResponse;
use elasticsearch::{
    Elasticsearch,
    http::{Method, StatusCode, headers::HeaderMap, headers::HeaderValue},
};
use eyre::{OptionExt, Result, eyre};
use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::{Value, json, value::RawValue};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle, time::sleep};
use url::Url;

const DEFAULT_BATCH_SIZE: usize = 5_000;
const DEFAULT_MAX_INFLIGHT_REQUESTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ElasticsearchOutputConfig {
    batch_size: usize,
    max_inflight_requests: usize,
}

#[derive(Clone, Debug)]
pub struct TemplateConfig {
    path: PathBuf,
    name: Option<String>,
    overwrite: bool,
}

impl TemplateConfig {
    pub fn try_new(
        path: Option<PathBuf>,
        name: Option<String>,
        overwrite: Option<bool>,
    ) -> Result<Option<Self>> {
        if path.is_none() {
            if name.is_some() {
                return Err(eyre!("--template-name requires --template"));
            }
            if overwrite.is_some() {
                return Err(eyre!("--template-overwrite requires --template"));
            }
            return Ok(None);
        }

        Ok(Some(Self {
            path: path.expect("checked above"),
            name,
            overwrite: overwrite.unwrap_or(true),
        }))
    }
}

impl ElasticsearchOutputConfig {
    pub const DEFAULT_BATCH_SIZE: usize = DEFAULT_BATCH_SIZE;
    pub const DEFAULT_MAX_INFLIGHT_REQUESTS: usize = DEFAULT_MAX_INFLIGHT_REQUESTS;

    pub fn try_new(batch_size: usize, max_inflight_requests: usize) -> Result<Self> {
        if batch_size == 0 {
            return Err(eyre!("batch size must be greater than zero"));
        }
        if max_inflight_requests == 0 {
            return Err(eyre!("max requests must be greater than zero"));
        }

        Ok(Self {
            batch_size,
            max_inflight_requests,
        })
    }

    fn channel_capacity(self) -> usize {
        self.batch_size
    }
}

impl Default for ElasticsearchOutputConfig {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            max_inflight_requests: DEFAULT_MAX_INFLIGHT_REQUESTS,
        }
    }
}

#[derive(Debug)]
pub struct ElasticsearchOutput {
    hostname: String,
    index: String,
    sender: Option<mpsc::Sender<Box<RawValue>>>,
    worker: JoinHandle<Result<usize>>,
}

impl ElasticsearchOutput {
    pub async fn try_new(
        client: Elasticsearch,
        url: Url,
        action: BulkAction,
        config: ElasticsearchOutputConfig,
        preflight: OutputPreflightConfig,
    ) -> Result<Self> {
        let hostname = url
            .host_str()
            .ok_or_eyre("Url missing host_str")?
            .to_string();
        let index = url.path().trim_start_matches('/').to_string();
        log::debug!("Elasticsearch output to {hostname}/{index}");

        let preflight = PreparedPreflight::try_from(preflight)?;
        preflight.run(&client, &index).await?;

        let client = Arc::new(client);
        let (sender, receiver) = mpsc::channel(config.channel_capacity());
        let worker = tokio::spawn(run_bulk_worker(
            Arc::clone(&client),
            hostname.clone(),
            index.clone(),
            action,
            config,
            preflight.bulk_pipeline,
            receiver,
        ));

        Ok(Self {
            hostname,
            index,
            sender: Some(sender),
            worker,
        })
    }
}

#[derive(Debug)]
struct ParsedTemplate {
    name: String,
    overwrite: bool,
    body: Value,
}

async fn install_template(
    client: &Elasticsearch,
    target_index: &str,
    parsed: &ParsedTemplate,
) -> Result<()> {
    warn_for_index_patterns(&parsed.body, target_index);

    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let path = format!("/_index_template/{}", parsed.name);
    let method = if parsed.overwrite {
        Method::Put
    } else {
        Method::Post
    };
    let params = if parsed.overwrite {
        None
    } else {
        Some(&[("create", "true")][..])
    };
    let body = serde_json::to_vec(&parsed.body)?;
    let response = client
        .send(method, &path, headers, params, Some(body), None)
        .await
        .map_err(|err| eyre!("failed to install index template '{}': {err}", parsed.name))?;
    let status = response.status_code();
    if !status.is_success() {
        let details = response
            .text()
            .await
            .unwrap_or_else(|err| format!("failed to read error body: {err}"));
        return Err(eyre!(
            "failed to install index template '{}': status {status}: {details}",
            parsed.name
        ));
    }

    Ok(())
}

fn parse_template(config: TemplateConfig) -> Result<ParsedTemplate> {
    let body = std::fs::read_to_string(&config.path)
        .map_err(|err| eyre!("failed to read template '{}': {err}", config.path.display()))?;
    let value = match config.path.extension().and_then(|ext| ext.to_str()) {
        Some("jsonc" | "json5") => serde_json5::from_str::<Value>(&body).map_err(|err| {
            eyre!(
                "failed to parse template '{}': {err}",
                config.path.display()
            )
        })?,
        _ => serde_json::from_str::<Value>(&body).map_err(|err| {
            eyre!(
                "failed to parse template '{}': {err}",
                config.path.display()
            )
        })?,
    };
    let name = match config.name {
        Some(name) => name,
        None => derive_template_name(&config.path)?,
    };
    if name.is_empty() {
        return Err(eyre!("template name must be non-empty"));
    }

    Ok(ParsedTemplate {
        name,
        overwrite: config.overwrite,
        body: value,
    })
}

fn derive_template_name(path: &Path) -> Result<String> {
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| eyre!("template name must be non-empty"))?;
    if name.is_empty() {
        return Err(eyre!("template name must be non-empty"));
    }
    Ok(name.to_string())
}

fn warn_for_index_patterns(template: &Value, target_index: &str) {
    match index_patterns_match(template, target_index) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("warning: template index_patterns do not match target index '{target_index}'")
        }
        Err(reason) => eprintln!(
            "warning: could not verify template index_patterns for target index '{target_index}': {reason}"
        ),
    }
}

fn index_patterns_match(template: &Value, target_index: &str) -> Result<bool> {
    let patterns = template
        .get("index_patterns")
        .ok_or_else(|| eyre!("index_patterns is missing"))?;
    let expressions = match patterns {
        Value::String(pattern) => vec![pattern.as_str()],
        Value::Array(patterns) => {
            let mut values = Vec::with_capacity(patterns.len());
            for pattern in patterns {
                values.push(
                    pattern
                        .as_str()
                        .ok_or_else(|| eyre!("index_patterns must contain only strings"))?,
                );
            }
            values
        }
        _ => return Err(eyre!("index_patterns must be a string or string array")),
    };

    let mut matched = false;
    for expression in expressions {
        for part in expression.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (exclude, pattern) = match part.strip_prefix('-') {
                Some("") => return Err(eyre!("invalid lone '-' index pattern")),
                Some(pattern) => (true, pattern),
                None => (false, part),
            };
            if wildcard_match(pattern, target_index) {
                matched = !exclude;
            }
        }
    }
    Ok(matched)
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let (mut pattern_index, mut value_index) = (0usize, 0usize);
    let mut star_index = None;
    let mut star_value_index = 0usize;

    while value_index < value.len() {
        if pattern_index < pattern.len() && pattern[pattern_index] == value[value_index] {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            star_value_index = value_index;
            pattern_index += 1;
        } else if let Some(star) = star_index {
            pattern_index = star + 1;
            star_value_index += 1;
            value_index = star_value_index;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
}

impl Sender for ElasticsearchOutput {
    async fn send(&mut self, value: Box<RawValue>) -> Result<usize> {
        let sender = self
            .sender
            .as_ref()
            .ok_or_eyre("Elasticsearch output already closed")?;
        sender
            .send(value)
            .await
            .map_err(|_| eyre!("Elasticsearch output worker closed unexpectedly"))?;
        Ok(0)
    }

    async fn close(mut self) -> Result<usize> {
        self.sender.take();
        self.worker.await.map_err(eyre::Report::new)?
    }
}

impl std::fmt::Display for ElasticsearchOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{}", self.hostname, self.index)
    }
}

async fn run_bulk_worker(
    client: Arc<Elasticsearch>,
    hostname: String,
    index: String,
    action: BulkAction,
    config: ElasticsearchOutputConfig,
    bulk_pipeline: Option<String>,
    mut receiver: mpsc::Receiver<Box<RawValue>>,
) -> Result<usize> {
    let mut batch = Vec::with_capacity(config.batch_size);
    let mut docs_sent = 0usize;
    let mut inflight = FuturesUnordered::<JoinHandle<Result<usize>>>::new();

    while let Some(doc) = receiver.recv().await {
        batch.push(doc);
        if batch.len() >= config.batch_size {
            spawn_flush(
                &mut inflight,
                &client,
                &hostname,
                &index,
                action,
                config,
                bulk_pipeline.as_deref(),
                &mut batch,
            )?;
            docs_sent +=
                reap_inflight_if_needed(&mut inflight, config.max_inflight_requests).await?;
        }
    }

    if !batch.is_empty() {
        spawn_flush(
            &mut inflight,
            &client,
            &hostname,
            &index,
            action,
            config,
            bulk_pipeline.as_deref(),
            &mut batch,
        )?;
    }

    while let Some(result) = inflight.next().await {
        docs_sent += result.map_err(eyre::Report::new)??;
    }

    Ok(docs_sent)
}

fn spawn_flush(
    inflight: &mut FuturesUnordered<JoinHandle<Result<usize>>>,
    client: &Arc<Elasticsearch>,
    hostname: &str,
    index: &str,
    action: BulkAction,
    config: ElasticsearchOutputConfig,
    bulk_pipeline: Option<&str>,
    batch: &mut Vec<Box<RawValue>>,
) -> Result<()> {
    let docs = std::mem::replace(batch, Vec::with_capacity(config.batch_size));
    let body = build_bulk_body(action, &docs)?;
    log::debug!("Bulk sending {} docs to {hostname}/{index}", docs.len());
    let client = Arc::clone(client);
    let index = index.to_string();
    let bulk_pipeline = bulk_pipeline.map(str::to_string);

    inflight.push(tokio::spawn(async move {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/x-ndjson"));
        let query = bulk_pipeline.as_ref().map(|pipeline| [("pipeline", pipeline.as_str())]);

        let mut attempt = 0u64;
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(30);

        loop {
            attempt += 1;
            let response = client
                .send(
                    Method::Post,
                    &format!("/{index}/_bulk"),
                    headers.clone(),
                    query.as_ref(),
                    Some(body.clone()),
                    None,
                )
                .await?;

            let status_code = response.status_code();
            let bulk_response = response.json::<BulkResponse>().await?;
            match status_code {
                StatusCode::BAD_REQUEST => {
                    log::error!(
                        "Bulk response: 400 - Bad request ({})",
                        bulk_response.error_cause()
                    );
                    return Ok(0);
                }
                StatusCode::TOO_MANY_REQUESTS => {
                    log::warn!(
                        "Bulk response: 429 - Too many requests (attempt {attempt}, backoff {:?}): {}",
                        backoff,
                        bulk_response.error_cause()
                    );
                    sleep(backoff).await;
                    if backoff < max_backoff {
                        backoff = std::cmp::min(backoff * 2, max_backoff);
                    }
                }
                _ => {
                    log::debug!("Bulk response status: {status_code}");
                    if bulk_response.has_errors() {
                        log::warn!(
                            "Bulk response contained errors: {}",
                            bulk_response.error_counts()
                        );
                    }
                    return Ok(bulk_response.success_count());
                }
            }
        }
    }));

    Ok(())
}

#[derive(Debug)]
struct PreparedPreflight {
    pipeline: Option<NamedJson>,
    template: Option<ParsedTemplate>,
    bulk_pipeline: Option<String>,
    template_pipeline: Option<String>,
}

#[derive(Debug)]
struct NamedJson {
    name: String,
    body: Value,
}

impl PreparedPreflight {
    fn try_from(config: OutputPreflightConfig) -> Result<Self> {
        let pipeline = match config.pipeline {
            Some(path) => Some(load_pipeline_json(
                "pipeline",
                &path,
                config.pipeline_name.as_deref(),
            )?),
            None => {
                if let Some(name) = config.pipeline_name.as_deref() {
                    if name == "_none" {
                        None
                    } else {
                        return Err(eyre!(
                            "--pipeline-name requires --pipeline unless the name is _none"
                        ));
                    }
                } else {
                    None
                }
            }
        };

        if pipeline
            .as_ref()
            .is_some_and(|pipeline| pipeline.name == "_none")
        {
            return Err(eyre!(
                "_none is reserved for the bulk pipeline target and cannot be installed as an ingest pipeline"
            ));
        }

        let template_config = TemplateConfig::try_new(
            config.template,
            config.template_name,
            config.template_overwrite,
        )?;
        let template = template_config.map(parse_template).transpose()?;

        let template_pipeline = template
            .as_ref()
            .and_then(|template| extract_default_pipeline(&template.body).map(str::to_string));

        if let (Some(template), Some(pipeline)) = (&template, &pipeline) {
            match template_pipeline.as_deref() {
                Some(name) if name == pipeline.name => {}
                Some(name) => {
                    return Err(eyre!(
                        "template references ingest pipeline '{name}', but --pipeline selects '{}'",
                        pipeline.name
                    ));
                }
                None => {
                    return Err(eyre!(
                        "template '{}' does not reference the provided pipeline '{}'",
                        template.name,
                        pipeline.name
                    ));
                }
            }
        }

        let bulk_pipeline = if template.is_none() {
            match (&pipeline, config.pipeline_name.as_deref()) {
                (Some(pipeline), _) => Some(pipeline.name.clone()),
                (None, Some("_none")) => Some("_none".to_string()),
                _ => None,
            }
        } else {
            None
        };

        Ok(Self {
            pipeline,
            template,
            bulk_pipeline,
            template_pipeline,
        })
    }

    async fn run(&self, client: &Elasticsearch, target_index: &str) -> Result<()> {
        if let Some(pipeline) = &self.pipeline {
            put_json(
                client,
                &format!("/_ingest/pipeline/{}", pipeline.name),
                &pipeline.body,
            )
            .await?;
        }

        if let (None, Some(pipeline_name)) = (&self.pipeline, &self.template_pipeline) {
            ensure_pipeline_exists(client, pipeline_name).await?;
        }

        if let Some(template) = &self.template {
            install_template(client, target_index, template).await?;
        }

        Ok(())
    }
}

fn load_pipeline_json(kind: &str, path: &Path, name_override: Option<&str>) -> Result<NamedJson> {
    if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
        return Err(eyre!(
            "{kind} file {} must use the .json extension",
            path.display()
        ));
    }
    let contents = fs::read_to_string(path)
        .map_err(|err| eyre!("failed to read {kind} file {}: {err}", path.display()))?;
    let body: Value = serde_json::from_str(&contents).map_err(|err| {
        eyre!(
            "failed to parse {kind} file {} as JSON: {err}",
            path.display()
        )
    })?;
    let name = match name_override {
        Some(name) => name.to_string(),
        None => path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string(),
    };
    if name.is_empty() {
        return Err(eyre!("{kind} name must be non-empty"));
    }
    Ok(NamedJson { name, body })
}

async fn put_json(client: &Elasticsearch, path: &str, body: &Value) -> Result<()> {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let body = serde_json::to_vec(body)?;
    let response = client
        .send(
            Method::Put,
            path,
            headers,
            Option::<&()>::None,
            Some(body),
            None,
        )
        .await?;
    ensure_success(response.status_code(), response.text().await?, path)
}

async fn ensure_pipeline_exists(client: &Elasticsearch, name: &str) -> Result<()> {
    let response = client
        .send(
            Method::Get,
            &format!("/_ingest/pipeline/{name}"),
            HeaderMap::new(),
            Option::<&()>::None,
            Option::<Vec<u8>>::None,
            None,
        )
        .await?;
    ensure_success(
        response.status_code(),
        response.text().await?,
        &format!("/_ingest/pipeline/{name}"),
    )
    .map_err(|err| {
        eyre!("template references missing or unavailable ingest pipeline '{name}': {err}")
    })
}

fn ensure_success(status: StatusCode, body: String, path: &str) -> Result<()> {
    if status.is_success() {
        Ok(())
    } else {
        Err(eyre!(
            "Elasticsearch request to {path} failed with status {status}: {body}"
        ))
    }
}

fn extract_default_pipeline(template: &Value) -> Option<&str> {
    let settings = template.get("template")?.get("settings")?;
    settings
        .get("index.default_pipeline")
        .and_then(Value::as_str)
        .or_else(|| {
            settings
                .get("index")
                .and_then(|index| index.get("default_pipeline"))
                .and_then(Value::as_str)
        })
}

async fn reap_inflight_if_needed(
    inflight: &mut FuturesUnordered<JoinHandle<Result<usize>>>,
    max_inflight_requests: usize,
) -> Result<usize> {
    let mut docs_sent = 0usize;
    while inflight.len() >= max_inflight_requests {
        if let Some(result) = inflight.next().await {
            docs_sent += result.map_err(eyre::Report::new)??;
        }
    }
    Ok(docs_sent)
}

fn build_bulk_body(action: BulkAction, batch: &[Box<RawValue>]) -> Result<Vec<u8>> {
    let mut body = Vec::with_capacity(batch.len() * 64);
    for doc in batch {
        match action {
            BulkAction::Create => {
                body.extend_from_slice(b"{\"create\":{}}\n");
                body.extend_from_slice(doc.get().as_bytes());
                body.push(b'\n');
            }
            BulkAction::Index => {
                body.extend_from_slice(b"{\"index\":{}}\n");
                body.extend_from_slice(doc.get().as_bytes());
                body.push(b'\n');
            }
            BulkAction::Update => append_update_operation(&mut body, doc)?,
        }
    }
    Ok(body)
}

fn append_update_operation(body: &mut Vec<u8>, doc: &RawValue) -> Result<()> {
    let (id, doc) = extract_update_id(doc)?;
    body.extend_from_slice(b"{\"update\":{\"_id\":");
    serde_json::to_writer(&mut *body, &id)?;
    body.extend_from_slice(b"}}\n");
    serde_json::to_writer(&mut *body, &json!({ "doc": doc }))?;
    body.push(b'\n');
    Ok(())
}

fn extract_update_id(doc: &RawValue) -> Result<(String, Value)> {
    match serde_json::from_str::<Value>(doc.get())? {
        Value::Object(mut map) => {
            let id_value = map
                .remove("_id")
                .ok_or_eyre("Update action requires an _id field on each document")?;
            let id = id_value
                .as_str()
                .ok_or_eyre("Update action requires _id to be a string")?
                .to_string();
            Ok((id, Value::Object(map)))
        }
        _ => Err(eyre!(
            "Update action requires each document to be a JSON object"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_BATCH_SIZE, DEFAULT_MAX_INFLIGHT_REQUESTS, ElasticsearchOutputConfig,
        OutputPreflightConfig, PreparedPreflight, TemplateConfig, build_bulk_body,
        extract_default_pipeline, extract_update_id, index_patterns_match, parse_template,
        wildcard_match,
    };
    use crate::output::BulkAction;
    use serde_json::{Value, json, value::RawValue};
    use std::{fs, path::PathBuf};

    fn temp_json_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "espipe-pipeline-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            name
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.json"));
        let _ = fs::remove_file(&path);
        path
    }

    #[test]
    fn build_bulk_body_uses_create_ndjson() {
        let docs = vec![
            RawValue::from_string("{\"a\":1}".to_string()).unwrap(),
            RawValue::from_string("{\"b\":2}".to_string()).unwrap(),
        ];

        let body = build_bulk_body(BulkAction::Create, &docs).unwrap();
        assert_eq!(
            String::from_utf8(body).unwrap(),
            "{\"create\":{}}\n{\"a\":1}\n{\"create\":{}}\n{\"b\":2}\n"
        );
    }

    #[test]
    fn build_bulk_body_uses_index_ndjson() {
        let docs = vec![RawValue::from_string("{\"a\":1}".to_string()).unwrap()];
        let body = build_bulk_body(BulkAction::Index, &docs).unwrap();
        assert_eq!(
            String::from_utf8(body).unwrap(),
            "{\"index\":{}}\n{\"a\":1}\n"
        );
    }

    #[test]
    fn build_bulk_body_wraps_update_docs() {
        let docs = vec![RawValue::from_string("{\"_id\":\"1\",\"a\":1}".to_string()).unwrap()];
        let body = build_bulk_body(BulkAction::Update, &docs).unwrap();
        let lines: Vec<Value> = String::from_utf8(body)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines[0]["update"]["_id"], "1");
        assert_eq!(lines[1], json!({ "doc": { "a": 1 } }));
    }

    #[test]
    fn extract_update_id_requires_id() {
        let doc = RawValue::from_string("{\"message\":\"hello\"}".to_string()).unwrap();
        let err = extract_update_id(&doc).err().expect("expected error");
        assert!(err.to_string().contains("_id"));
    }

    #[test]
    fn default_worker_limits_are_bounded() {
        let config = ElasticsearchOutputConfig::default();
        assert_eq!(config.batch_size, DEFAULT_BATCH_SIZE);
        assert_eq!(config.channel_capacity(), DEFAULT_BATCH_SIZE);
        assert_eq!(config.max_inflight_requests, DEFAULT_MAX_INFLIGHT_REQUESTS);
    }

    #[test]
    fn config_rejects_zero_limits() {
        let batch_err = ElasticsearchOutputConfig::try_new(0, 1).unwrap_err();
        assert!(batch_err.to_string().contains("batch size"));

        let requests_err = ElasticsearchOutputConfig::try_new(1, 0).unwrap_err();
        assert!(requests_err.to_string().contains("max requests"));
    }

    #[test]
    fn template_name_defaults_to_file_stem() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs-docs.json");
        std::fs::write(&path, r#"{"index_patterns":["logs-*"]}"#).unwrap();

        let parsed = parse_template(TemplateConfig {
            path,
            name: None,
            overwrite: true,
        })
        .unwrap();

        assert_eq!(parsed.name, "logs-docs");
        assert!(parsed.overwrite);
    }

    #[test]
    fn template_name_override_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs-docs.json");
        std::fs::write(&path, r#"{"index_patterns":["logs-*"]}"#).unwrap();

        let parsed = parse_template(TemplateConfig {
            path,
            name: Some("custom-template".to_string()),
            overwrite: false,
        })
        .unwrap();

        assert_eq!(parsed.name, "custom-template");
        assert!(!parsed.overwrite);
    }

    #[test]
    fn template_name_rejects_empty_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs-docs.json");
        std::fs::write(&path, r#"{"index_patterns":["logs-*"]}"#).unwrap();

        let err = parse_template(TemplateConfig {
            path,
            name: Some(String::new()),
            overwrite: true,
        })
        .unwrap_err();

        assert!(err.to_string().contains("template name must be non-empty"));
    }

    #[test]
    fn strict_json_template_rejects_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("template.json");
        std::fs::write(&path, r#"{"index_patterns":["logs-*"] /* no */}"#).unwrap();

        let err = parse_template(TemplateConfig {
            path: path.clone(),
            name: None,
            overwrite: true,
        })
        .unwrap_err();

        assert!(err.to_string().contains(&path.display().to_string()));
    }

    #[test]
    fn jsonc_and_json5_templates_are_normalized() {
        let dir = tempfile::tempdir().unwrap();
        let jsonc_path = dir.path().join("template.jsonc");
        std::fs::write(
            &jsonc_path,
            r#"{"index_patterns":["logs-*"], /* comment */ "priority": 1}"#,
        )
        .unwrap();
        let json5_path = dir.path().join("template.json5");
        std::fs::write(
            &json5_path,
            r#"{index_patterns:["logs-*"], template: { settings: { number_of_shards: 1 } }}"#,
        )
        .unwrap();

        let jsonc = parse_template(TemplateConfig {
            path: jsonc_path,
            name: None,
            overwrite: true,
        })
        .unwrap();
        let json5 = parse_template(TemplateConfig {
            path: json5_path,
            name: None,
            overwrite: true,
        })
        .unwrap();

        assert_eq!(jsonc.body["priority"], 1);
        assert_eq!(json5.body["template"]["settings"]["number_of_shards"], 1);
    }

    #[test]
    fn index_patterns_follow_multi_target_ordering() {
        assert!(index_patterns_match(&json!({"index_patterns":"test*"}), "test3").unwrap());
        assert!(!index_patterns_match(&json!({"index_patterns":"test*,-test3"}), "test3").unwrap());
        assert!(
            index_patterns_match(&json!({"index_patterns":"test3*,-test3,test*"}), "test3")
                .unwrap()
        );
        assert!(index_patterns_match(&json!({"index_patterns":["logs-*"]}), "logs-docs").unwrap());
        assert!(
            !index_patterns_match(&json!({"index_patterns":["metrics-*"]}), "logs-docs").unwrap()
        );
        assert!(index_patterns_match(&json!({"index_patterns":"*"}), "logs-docs").unwrap());
    }

    #[test]
    fn index_patterns_report_unverifiable_shapes() {
        assert!(index_patterns_match(&json!({}), "logs-docs").is_err());
        assert!(index_patterns_match(&json!({"index_patterns": 1}), "logs-docs").is_err());
        assert!(index_patterns_match(&json!({"index_patterns": "-"}), "logs-docs").is_err());
    }

    #[test]
    fn wildcard_matching_supports_zero_or_more_chars() {
        assert!(wildcard_match("logs-*", "logs-docs"));
        assert!(wildcard_match("logs*", "logs"));
        assert!(wildcard_match("*docs", "logs-docs"));
        assert!(!wildcard_match("metrics-*", "logs-docs"));
    }

    #[test]
    fn prepared_preflight_derives_pipeline_name_and_bulk_target() {
        let path = temp_json_path("geoip");
        fs::write(&path, r#"{"processors":[]}"#).unwrap();

        let preflight = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(path.clone()),
            ..OutputPreflightConfig::default()
        })
        .unwrap();

        assert_eq!(preflight.pipeline.as_ref().unwrap().name, "geoip");
        assert_eq!(preflight.bulk_pipeline.as_deref(), Some("geoip"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn prepared_preflight_applies_pipeline_name_override() {
        let path = temp_json_path("derived");
        fs::write(&path, r#"{"processors":[]}"#).unwrap();

        let preflight = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(path.clone()),
            pipeline_name: Some("normalized".to_string()),
            ..OutputPreflightConfig::default()
        })
        .unwrap();

        assert_eq!(preflight.pipeline.as_ref().unwrap().name, "normalized");
        assert_eq!(preflight.bulk_pipeline.as_deref(), Some("normalized"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn prepared_preflight_allows_none_without_pipeline_file() {
        let preflight = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline_name: Some("_none".to_string()),
            ..OutputPreflightConfig::default()
        })
        .unwrap();

        assert!(preflight.pipeline.is_none());
        assert_eq!(preflight.bulk_pipeline.as_deref(), Some("_none"));
    }

    #[test]
    fn prepared_preflight_rejects_pipeline_name_without_pipeline_file() {
        let err = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline_name: Some("geoip".to_string()),
            ..OutputPreflightConfig::default()
        })
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--pipeline-name requires --pipeline")
        );
    }

    #[test]
    fn prepared_preflight_rejects_invalid_pipeline_json() {
        let path = temp_json_path("invalid");
        fs::write(&path, "{").unwrap();

        let err = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(path.clone()),
            ..OutputPreflightConfig::default()
        })
        .unwrap_err();

        assert!(err.to_string().contains("failed to parse pipeline file"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn prepared_preflight_rejects_non_json_pipeline_extension() {
        let path = std::env::temp_dir().join(format!(
            "espipe-pipeline-test-{}-pipeline.jsonc",
            std::process::id()
        ));
        fs::write(&path, r#"{"processors":[]}"#).unwrap();

        let err = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(path.clone()),
            ..OutputPreflightConfig::default()
        })
        .unwrap_err();

        assert!(err.to_string().contains(".json extension"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn extract_default_pipeline_supports_nested_and_flattened_settings() {
        let nested = json!({
            "template": {
                "settings": {
                    "index": {
                        "default_pipeline": "geoip"
                    }
                }
            }
        });
        assert_eq!(extract_default_pipeline(&nested), Some("geoip"));

        let flattened = json!({
            "template": {
                "settings": {
                    "index.default_pipeline": "normalized"
                }
            }
        });
        assert_eq!(extract_default_pipeline(&flattened), Some("normalized"));
    }

    #[test]
    fn prepared_preflight_rejects_template_pipeline_mismatch_before_requests() {
        let pipeline_path = temp_json_path("geoip");
        let template_path = temp_json_path("template");
        fs::write(&pipeline_path, r#"{"processors":[]}"#).unwrap();
        fs::write(
            &template_path,
            r#"{"template":{"settings":{"index.default_pipeline":"other"}}}"#,
        )
        .unwrap();

        let err = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(pipeline_path.clone()),
            template: Some(template_path.clone()),
            ..OutputPreflightConfig::default()
        })
        .unwrap_err();

        assert!(err.to_string().contains("other"));
        assert!(err.to_string().contains("geoip"));

        let _ = fs::remove_file(pipeline_path);
        let _ = fs::remove_file(template_path);
    }

    #[test]
    fn prepared_preflight_template_with_pipeline_omits_bulk_pipeline_target() {
        let pipeline_path = temp_json_path("geoip");
        let template_path = temp_json_path("template-geoip");
        fs::write(&pipeline_path, r#"{"processors":[]}"#).unwrap();
        fs::write(
            &template_path,
            r#"{"template":{"settings":{"index.default_pipeline":"geoip"}}}"#,
        )
        .unwrap();

        let preflight = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(pipeline_path.clone()),
            template: Some(template_path.clone()),
            ..OutputPreflightConfig::default()
        })
        .unwrap();

        assert_eq!(preflight.pipeline.as_ref().unwrap().name, "geoip");
        assert_eq!(preflight.template_pipeline.as_deref(), Some("geoip"));
        assert!(preflight.bulk_pipeline.is_none());

        let _ = fs::remove_file(pipeline_path);
        let _ = fs::remove_file(template_path);
    }
}
