mod bulk_response;

use super::{BulkAction, Sender};
use bulk_response::BulkResponse;
use elasticsearch::{
    Elasticsearch,
    http::{Method, StatusCode, headers::HeaderMap, headers::HeaderValue},
};
use eyre::{OptionExt, Result, eyre};
use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::{Value, json, value::RawValue};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::sleep,
};
use url::Url;

const DEFAULT_BATCH_SIZE: usize = 5_000;
const DEFAULT_MAX_INFLIGHT_REQUESTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ElasticsearchOutputConfig {
    batch_size: usize,
    max_inflight_requests: usize,
}

impl ElasticsearchOutputConfig {
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
    pub fn try_new(
        client: Elasticsearch,
        url: Url,
        action: BulkAction,
        config: ElasticsearchOutputConfig,
    ) -> Result<Self> {
        let hostname = url
            .host_str()
            .ok_or_eyre("Url missing host_str")?
            .to_string();
        let index = url.path().trim_start_matches('/').to_string();
        log::debug!("Elasticsearch output to {hostname}/{index}");

        let client = Arc::new(client);
        let (sender, receiver) = mpsc::channel(config.channel_capacity());
        let worker = tokio::spawn(run_bulk_worker(
            Arc::clone(&client),
            hostname.clone(),
            index.clone(),
            action,
            config,
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
    mut receiver: mpsc::Receiver<Box<RawValue>>,
) -> Result<usize> {
    let mut batch = Vec::with_capacity(config.batch_size);
    let mut docs_sent = 0usize;
    let mut inflight = FuturesUnordered::<JoinHandle<Result<usize>>>::new();

    while let Some(doc) = receiver.recv().await {
        batch.push(doc);
        if batch.len() >= config.batch_size {
            spawn_flush(&mut inflight, &client, &hostname, &index, action, config, &mut batch)?;
            docs_sent += reap_inflight_if_needed(&mut inflight, config.max_inflight_requests).await?;
        }
    }

    if !batch.is_empty() {
        spawn_flush(&mut inflight, &client, &hostname, &index, action, config, &mut batch)?;
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
    batch: &mut Vec<Box<RawValue>>,
) -> Result<()> {
    let docs = std::mem::replace(batch, Vec::with_capacity(config.batch_size));
    let body = build_bulk_body(action, &docs)?;
    log::debug!("Bulk sending {} docs to {hostname}/{index}", docs.len());
    let client = Arc::clone(client);
    let index = index.to_string();

    inflight.push(tokio::spawn(async move {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/x-ndjson"));

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
                    Option::<&()>::None,
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
        build_bulk_body, extract_update_id,
    };
    use crate::output::BulkAction;
    use serde_json::{Value, json, value::RawValue};

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
        let docs = vec![
            RawValue::from_string("{\"_id\":\"1\",\"a\":1}".to_string()).unwrap(),
        ];
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
}
