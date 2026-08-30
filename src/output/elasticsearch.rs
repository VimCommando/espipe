mod bulk_response;
mod embedded_templates;

use super::{BulkAction, Sender};
use crate::input::InputDocument;
use crate::output::OutputPreflightConfig;
use bulk_response::BulkResponse;
use elasticsearch::{
    Elasticsearch,
    http::{Method, StatusCode, headers::HeaderMap, headers::HeaderValue},
};
use eyre::{OptionExt, Result, eyre};
use futures::{StreamExt, stream::FuturesUnordered};
#[cfg(test)]
use serde_json::value::RawValue;
use serde_json::{Map, Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle, time::sleep};
use url::Url;

const DEFAULT_BATCH_SIZE: usize = 5_000;
const MULTI_SOURCE_DEFAULT_BATCH_SIZE: usize = 500;
const DEFAULT_MAX_INFLIGHT_REQUESTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ElasticsearchOutputConfig {
    batch_size: usize,
    max_inflight_requests: usize,
}

#[derive(Clone, Debug)]
pub struct TemplateConfig {
    source: TemplateSource,
    name: Option<String>,
    overwrite: bool,
}

#[derive(Clone, Debug)]
enum TemplateSource {
    File(PathBuf),
    Bundled(String),
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

        let path = path.expect("checked above");
        let source = match path.to_str() {
            Some(selector) if selector.starts_with('_') => {
                TemplateSource::Bundled(selector.to_string())
            }
            _ => TemplateSource::File(path),
        };

        Ok(Some(Self {
            source,
            name,
            overwrite: overwrite.unwrap_or(true),
        }))
    }
}

pub(super) fn validate_bundled_template(path: &Path) -> Result<()> {
    if let Some(selector) = path.to_str().filter(|value| value.starts_with('_')) {
        embedded_templates::resolve(selector)?;
    }
    Ok(())
}

impl ElasticsearchOutputConfig {
    pub const DEFAULT_BATCH_SIZE: usize = DEFAULT_BATCH_SIZE;
    pub const MULTI_SOURCE_DEFAULT_BATCH_SIZE: usize = MULTI_SOURCE_DEFAULT_BATCH_SIZE;
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
    sender: Option<mpsc::Sender<InputDocument>>,
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
    bundled: bool,
}

async fn install_template(
    client: &Elasticsearch,
    target_index: &str,
    parsed: &ParsedTemplate,
) -> Result<()> {
    if parsed.bundled {
        return install_bundled_template(client, target_index, parsed).await;
    }

    warn_for_index_patterns(&parsed.body, target_index);
    write_template(client, parsed, &parsed.body).await
}

async fn install_bundled_template(
    client: &Elasticsearch,
    target_index: &str,
    parsed: &ParsedTemplate,
) -> Result<()> {
    let path = format!("/_index_template/{}", parsed.name);
    let response = client
        .send(
            Method::Get,
            &path,
            HeaderMap::new(),
            Option::<&()>::None,
            Option::<Vec<u8>>::None,
            None,
        )
        .await
        .map_err(|err| eyre!("failed to look up index template '{}': {err}", parsed.name))?;

    match response.status_code() {
        StatusCode::NOT_FOUND => {
            let mut body = parsed.body.clone();
            append_exact_index(&mut body, target_index).map_err(|err| {
                eyre!(
                    "bundled template '{}' cannot be installed: {err}",
                    parsed.name
                )
            })?;
            write_template(client, parsed, &body).await
        }
        status if status.is_success() => {
            let response_body = response.json::<Value>().await.map_err(|err| {
                eyre!(
                    "failed to parse index template lookup response for '{}': {err}",
                    parsed.name
                )
            })?;
            let mut stored = extract_stored_template(&response_body, &parsed.name)?;
            if !append_exact_index(&mut stored, target_index)? {
                return Ok(());
            }
            if !parsed.overwrite {
                return Err(eyre!(
                    "index template '{}' does not list target index '{target_index}'; appending it requires --template-overwrite=true",
                    parsed.name
                ));
            }
            write_template(client, parsed, &stored).await
        }
        status => {
            let details = response
                .text()
                .await
                .unwrap_or_else(|err| format!("failed to read error body: {err}"));
            Err(eyre!(
                "failed to look up index template '{}': status {status}: {details}",
                parsed.name
            ))
        }
    }
}

fn extract_stored_template(response: &Value, selected_name: &str) -> Result<Value> {
    let entries = response
        .get("index_templates")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            eyre!("index template lookup for '{selected_name}' has no index_templates array")
        })?;
    let matches = entries
        .iter()
        .filter(|entry| entry.get("name").and_then(Value::as_str) == Some(selected_name))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(eyre!(
            "index template lookup for '{selected_name}' returned {} exact matches; expected one",
            matches.len()
        ));
    }
    let body = matches[0]
        .get("index_template")
        .filter(|body| body.is_object())
        .ok_or_else(|| {
            eyre!("index template lookup for '{selected_name}' has no composable template body")
        })?
        .clone();
    validate_index_patterns_array(&body)?;
    Ok(body)
}

fn validate_index_patterns_array(template: &Value) -> Result<()> {
    let patterns = template
        .get("index_patterns")
        .and_then(Value::as_array)
        .ok_or_else(|| eyre!("index_patterns must be an array of strings"))?;
    if patterns.iter().any(|pattern| !pattern.is_string()) {
        return Err(eyre!("index_patterns must be an array of strings"));
    }
    Ok(())
}

fn append_exact_index(template: &mut Value, target_index: &str) -> Result<bool> {
    validate_index_patterns_array(template)?;
    let patterns = template
        .get_mut("index_patterns")
        .and_then(Value::as_array_mut)
        .expect("validated above");
    if patterns
        .iter()
        .any(|pattern| pattern.as_str() == Some(target_index))
    {
        return Ok(false);
    }
    patterns.push(Value::String(target_index.to_string()));
    Ok(true)
}

async fn write_template(
    client: &Elasticsearch,
    parsed: &ParsedTemplate,
    body: &Value,
) -> Result<()> {
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
    let body = serde_json::to_vec(body)?;
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
    let (body, default_name, bundled) = match config.source {
        TemplateSource::File(path) => {
            let contents = std::fs::read_to_string(&path)
                .map_err(|err| eyre!("failed to read template '{}': {err}", path.display()))?;
            let body = parse_config_body("template", &path, &contents)?;
            let name = derive_template_name(&path)?;
            (body, name, false)
        }
        TemplateSource::Bundled(selector) => {
            let embedded = embedded_templates::resolve(&selector)?;
            (embedded.body, embedded.default_name, true)
        }
    };
    let name = match config.name {
        Some(name) => name,
        None => default_name,
    };
    if name.is_empty() {
        return Err(eyre!("template name must be non-empty"));
    }

    Ok(ParsedTemplate {
        name,
        overwrite: config.overwrite,
        body,
        bundled,
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
    async fn send(&mut self, value: InputDocument) -> Result<usize> {
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
    mut receiver: mpsc::Receiver<InputDocument>,
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
    batch: &mut Vec<InputDocument>,
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
            Some(path) => Some(load_pipeline_config(
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

fn load_pipeline_config(kind: &str, path: &Path, name_override: Option<&str>) -> Result<NamedJson> {
    let contents = fs::read_to_string(path)
        .map_err(|err| eyre!("failed to read {kind} file {}: {err}", path.display()))?;
    let body = parse_pipeline_body(kind, path, &contents)?;
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

fn parse_pipeline_body(kind: &str, path: &Path, body: &str) -> Result<Value> {
    match normalized_extension(path).as_deref() {
        Some("json") => serde_json::from_str::<Value>(body).map_err(|err| {
            eyre!(
                "failed to parse {kind} file {} as JSON: {err}",
                path.display()
            )
        }),
        Some("yml" | "yaml") => yaml_serde::from_str::<Value>(body).map_err(|err| {
            eyre!(
                "failed to parse {kind} file {} as YAML: {err}",
                path.display()
            )
        }),
        _ => Err(eyre!(
            "{kind} file {} must use the .json, .yml, or .yaml extension",
            path.display()
        )),
    }
}

fn parse_config_body(kind: &str, path: &Path, body: &str) -> Result<Value> {
    match normalized_extension(path).as_deref() {
        Some("json") => serde_json::from_str::<Value>(body).map_err(|err| {
            eyre!(
                "failed to parse {kind} file {} as JSON: {err}",
                path.display()
            )
        }),
        Some("jsonc" | "json5") => serde_json5::from_str::<Value>(body)
            .map_err(|err| eyre!("failed to parse {kind} file {}: {err}", path.display())),
        Some("yml" | "yaml") => yaml_serde::from_str::<Value>(body).map_err(|err| {
            eyre!(
                "failed to parse {kind} file {} as YAML: {err}",
                path.display()
            )
        }),
        _ => serde_json::from_str::<Value>(body).map_err(|err| {
            eyre!(
                "failed to parse {kind} file {} as JSON: {err}",
                path.display()
            )
        }),
    }
}

fn normalized_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
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

fn build_bulk_body(action: BulkAction, batch: &[InputDocument]) -> Result<Vec<u8>> {
    let mut body = Vec::with_capacity(batch.len() * 64);
    for doc in batch {
        match action {
            BulkAction::Create => append_document_operation(&mut body, "create", doc)?,
            BulkAction::Index => append_document_operation(&mut body, "index", doc)?,
            BulkAction::Update => append_update_operation(&mut body, doc, false)?,
            BulkAction::Upsert => append_update_operation(&mut body, doc, true)?,
        }
    }
    Ok(body)
}

fn append_document_operation(
    body: &mut Vec<u8>,
    action: &str,
    document: &InputDocument,
) -> Result<()> {
    let (id, source) = extract_document(document)?;
    append_metadata(body, action, id.as_deref())?;
    serde_json::to_writer(&mut *body, &source)?;
    body.push(b'\n');
    Ok(())
}

fn append_update_operation(
    body: &mut Vec<u8>,
    document: &InputDocument,
    doc_as_upsert: bool,
) -> Result<()> {
    let (id, source) = extract_document(document)?;
    let id = id.ok_or_else(|| {
        if doc_as_upsert {
            eyre!(
                "Upsert action requires a document ID (explicit _id or generated ID) on each document"
            )
        } else {
            eyre!(
                "Update action requires a document ID (explicit _id or generated ID) on each document"
            )
        }
    })?;
    append_metadata(body, "update", Some(&id))?;
    let payload = if doc_as_upsert {
        json!({ "doc": source, "doc_as_upsert": true })
    } else {
        json!({ "doc": source })
    };
    serde_json::to_writer(&mut *body, &payload)?;
    body.push(b'\n');
    Ok(())
}

fn append_metadata(body: &mut Vec<u8>, action: &str, id: Option<&str>) -> Result<()> {
    let mut action_metadata = Map::new();
    if let Some(id) = id {
        action_metadata.insert("_id".to_string(), Value::String(id.to_string()));
    }
    let mut metadata = Map::new();
    metadata.insert(action.to_string(), Value::Object(action_metadata));
    serde_json::to_writer(&mut *body, &metadata)?;
    body.push(b'\n');
    Ok(())
}

fn extract_document(document: &InputDocument) -> Result<(Option<String>, Value)> {
    match serde_json::from_str::<Value>(document.raw.get())? {
        Value::Object(mut map) => {
            let id = match map.remove("_id") {
                Some(value) => Some(
                    value
                        .as_str()
                        .ok_or_else(|| eyre!("_id must be a string"))?
                        .to_string(),
                ),
                None => document.generated_id.clone(),
            };
            Ok((id, Value::Object(map)))
        }
        _ => Err(eyre!("Each document must be a JSON object")),
    }
}

#[cfg(test)]
fn extract_update_id(doc: &RawValue) -> Result<(String, Value)> {
    let document = InputDocument {
        raw: RawValue::from_string(doc.get().to_string())?,
        generated_id: None,
    };
    let (id, source) = extract_document(&document)?;
    let id = id.ok_or_else(|| {
        eyre!(
            "Update action requires a document ID (explicit _id or generated ID) on each document"
        )
    })?;
    Ok((id, source))
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_BATCH_SIZE, DEFAULT_MAX_INFLIGHT_REQUESTS, ElasticsearchOutputConfig,
        OutputPreflightConfig, PreparedPreflight, TemplateConfig, TemplateSource,
        append_exact_index, build_bulk_body, extract_default_pipeline, extract_stored_template,
        extract_update_id, index_patterns_match, parse_template, wildcard_match,
    };
    use crate::input::InputDocument;
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

    fn document(raw: &str) -> InputDocument {
        InputDocument {
            raw: RawValue::from_string(raw.to_string()).unwrap(),
            generated_id: None,
        }
    }

    fn generated_document(raw: &str, id: &str) -> InputDocument {
        InputDocument {
            raw: RawValue::from_string(raw.to_string()).unwrap(),
            generated_id: Some(id.to_string()),
        }
    }

    #[test]
    fn build_bulk_body_uses_create_ndjson() {
        let docs = vec![document("{\"a\":1}"), document("{\"b\":2}")];

        let body = build_bulk_body(BulkAction::Create, &docs).unwrap();
        assert_eq!(
            String::from_utf8(body).unwrap(),
            "{\"create\":{}}\n{\"a\":1}\n{\"create\":{}}\n{\"b\":2}\n"
        );
    }

    #[test]
    fn build_bulk_body_uses_index_ndjson() {
        let docs = vec![document("{\"a\":1}")];
        let body = build_bulk_body(BulkAction::Index, &docs).unwrap();
        assert_eq!(
            String::from_utf8(body).unwrap(),
            "{\"index\":{}}\n{\"a\":1}\n"
        );
    }

    #[test]
    fn create_and_index_move_explicit_id_to_metadata() {
        let docs = vec![document("{\"_id\":\"provided\",\"a\":1}")];

        let create =
            String::from_utf8(build_bulk_body(BulkAction::Create, &docs).unwrap()).unwrap();
        assert_eq!(create, "{\"create\":{\"_id\":\"provided\"}}\n{\"a\":1}\n");

        let index = String::from_utf8(build_bulk_body(BulkAction::Index, &docs).unwrap()).unwrap();
        assert_eq!(index, "{\"index\":{\"_id\":\"provided\"}}\n{\"a\":1}\n");
    }

    #[test]
    fn build_bulk_body_wraps_update_docs() {
        let docs = vec![document("{\"_id\":\"1\",\"a\":1}")];
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
    fn build_bulk_body_wraps_upsert_docs() {
        let docs = vec![document("{\"_id\":\"1\",\"a\":1}")];
        let body = build_bulk_body(BulkAction::Upsert, &docs).unwrap();
        let lines: Vec<Value> = String::from_utf8(body)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines[0], json!({ "update": { "_id": "1" } }));
        assert_eq!(
            lines[1],
            json!({ "doc": { "a": 1 }, "doc_as_upsert": true })
        );
    }

    #[test]
    fn generated_id_is_used_when_source_has_no_explicit_id() {
        let docs = vec![generated_document("{\"a\":1}", "generated")];
        let body = String::from_utf8(build_bulk_body(BulkAction::Index, &docs).unwrap()).unwrap();
        assert_eq!(body, "{\"index\":{\"_id\":\"generated\"}}\n{\"a\":1}\n");
    }

    #[test]
    fn extract_update_id_requires_id() {
        let doc = RawValue::from_string("{\"message\":\"hello\"}".to_string()).unwrap();
        let err = extract_update_id(&doc).err().expect("expected error");
        assert!(err.to_string().contains("_id"));
    }

    #[test]
    fn non_string_ids_are_rejected() {
        let docs = vec![document("{\"_id\":42,\"a\":1}")];
        let err = build_bulk_body(BulkAction::Update, &docs).unwrap_err();
        assert!(err.to_string().contains("_id must be a string"));
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
            source: TemplateSource::File(path),
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
            source: TemplateSource::File(path),
            name: Some("custom-template".to_string()),
            overwrite: false,
        })
        .unwrap();

        assert_eq!(parsed.name, "custom-template");
        assert!(!parsed.overwrite);
    }

    #[test]
    fn bundled_template_uses_default_and_overridden_names() {
        let default = parse_template(
            TemplateConfig::try_new(Some(PathBuf::from("_okf")), None, None)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(default.name, "open-knowledge-format");
        assert!(default.bundled);

        let overridden = parse_template(
            TemplateConfig::try_new(
                Some(PathBuf::from("_okf")),
                Some("team-okf".to_string()),
                None,
            )
            .unwrap()
            .unwrap(),
        )
        .unwrap();
        assert_eq!(overridden.name, "team-okf");
        assert!(overridden.bundled);
    }

    #[test]
    fn template_paths_with_underscores_outside_the_first_character_stay_files() {
        let config =
            TemplateConfig::try_new(Some(PathBuf::from("templates/_okf.json")), None, None)
                .unwrap()
                .unwrap();
        assert!(matches!(config.source, TemplateSource::File(_)));
    }

    #[test]
    fn stored_template_extraction_requires_one_exact_valid_body() {
        let body = json!({
            "index_templates": [{
                "name": "open-knowledge-format",
                "index_template": {"index_patterns": ["knowledge-a"], "priority": 7}
            }]
        });
        let stored = extract_stored_template(&body, "open-knowledge-format").unwrap();
        assert_eq!(stored["priority"], 7);

        assert!(extract_stored_template(&json!({}), "open-knowledge-format").is_err());
        assert!(
            extract_stored_template(
                &json!({"index_templates": [{"name": "other", "index_template": {"index_patterns": []}}]}),
                "open-knowledge-format"
            )
            .is_err()
        );
        assert!(
            extract_stored_template(
                &json!({"index_templates": [{"name": "open-knowledge-format", "index_template": {"index_patterns": "knowledge-*"}}]}),
                "open-knowledge-format"
            )
            .is_err()
        );
    }

    #[test]
    fn exact_target_append_ignores_wildcard_coverage() {
        let mut body = json!({"index_patterns": ["team-*"]});
        assert!(append_exact_index(&mut body, "team-knowledge").unwrap());
        assert_eq!(body["index_patterns"], json!(["team-*", "team-knowledge"]));
        assert!(!append_exact_index(&mut body, "team-knowledge").unwrap());
    }

    #[test]
    fn template_name_rejects_empty_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs-docs.json");
        std::fs::write(&path, r#"{"index_patterns":["logs-*"]}"#).unwrap();

        let err = parse_template(TemplateConfig {
            source: TemplateSource::File(path),
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
            source: TemplateSource::File(path.clone()),
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
            source: TemplateSource::File(jsonc_path),
            name: None,
            overwrite: true,
        })
        .unwrap();
        let json5 = parse_template(TemplateConfig {
            source: TemplateSource::File(json5_path),
            name: None,
            overwrite: true,
        })
        .unwrap();

        assert_eq!(jsonc.body["priority"], 1);
        assert_eq!(json5.body["template"]["settings"]["number_of_shards"], 1);
    }

    #[test]
    fn yaml_template_is_normalized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs-docs.YML");
        std::fs::write(
            &path,
            r#"
index_patterns:
  - logs-*
template:
  settings:
    number_of_shards: 1
"#,
        )
        .unwrap();

        let parsed = parse_template(TemplateConfig {
            source: TemplateSource::File(path),
            name: None,
            overwrite: true,
        })
        .unwrap();

        assert_eq!(parsed.name, "logs-docs");
        assert_eq!(parsed.body["index_patterns"][0], "logs-*");
        assert_eq!(parsed.body["template"]["settings"]["number_of_shards"], 1);
    }

    #[test]
    fn template_unknown_extension_falls_back_to_strict_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs-docs.txt");
        std::fs::write(&path, r#"{"index_patterns":["logs-*"]}"#).unwrap();

        let parsed = parse_template(TemplateConfig {
            source: TemplateSource::File(path),
            name: None,
            overwrite: true,
        })
        .unwrap();

        assert_eq!(parsed.body["index_patterns"][0], "logs-*");
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
    fn prepared_preflight_accepts_yaml_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("geoip.YML");
        fs::write(
            &path,
            "processors:\n  - set:\n      field: normalized\n      value: true\n",
        )
        .unwrap();

        let preflight = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(path),
            ..OutputPreflightConfig::default()
        })
        .unwrap();

        assert_eq!(preflight.pipeline.as_ref().unwrap().name, "geoip");
        assert_eq!(
            preflight.pipeline.as_ref().unwrap().body["processors"][0]["set"]["field"],
            "normalized"
        );
    }

    #[test]
    fn prepared_preflight_rejects_invalid_pipeline_yaml() {
        let path = std::env::temp_dir().join(format!(
            "espipe-pipeline-test-{}-pipeline.yml",
            std::process::id()
        ));
        fs::write(&path, "processors: [").unwrap();

        let err = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(path.clone()),
            ..OutputPreflightConfig::default()
        })
        .unwrap_err();

        assert!(err.to_string().contains("failed to parse pipeline file"));
        assert!(err.to_string().contains("as YAML"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn prepared_preflight_rejects_unsupported_pipeline_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pipeline.jsonc");
        fs::write(&path, r#"{"processors":[]}"#).unwrap();

        let err = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(path),
            ..OutputPreflightConfig::default()
        })
        .unwrap_err();

        assert!(err.to_string().contains(".json, .yml, or .yaml"));
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
