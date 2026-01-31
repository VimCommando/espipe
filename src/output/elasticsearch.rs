mod bulk_response;

use super::{BulkAction, Sender};
use bulk_response::BulkResponse;
use elasticsearch::{BulkOperation, BulkParts, Elasticsearch, http::StatusCode};
use eyre::{OptionExt, Result, eyre};
use futures::{future::join_all, stream::FuturesUnordered};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use url::Url;

static BATCH_SIZE: usize = 5_000;

#[derive(Debug)]
pub struct ElasticsearchOutput {
    client: Arc<Elasticsearch>,
    hostname: String,
    index: String,
    action: BulkAction,
    queue: Vec<Value>,
    futures: FuturesUnordered<JoinHandle<Result<usize>>>,
}

impl ElasticsearchOutput {
    pub fn try_new(client: Elasticsearch, url: Url, action: BulkAction) -> Result<Self> {
        let hostname = url
            .host_str()
            .ok_or_eyre("Url missing host_str")?
            .to_string();
        let index = url.path().trim_start_matches('/').to_string();
        log::debug!("Elasticsearch output to {hostname}/{index}");
        Ok(Self {
            client: Arc::new(client),
            hostname,
            index,
            action,
            queue: Vec::with_capacity(BATCH_SIZE),
            futures: FuturesUnordered::new(),
        })
    }

    async fn flush(&mut self) -> Result<()> {
        let batch_size = match self.queue.len() >= BATCH_SIZE {
            true => BATCH_SIZE,
            false => self.queue.len(),
        };
        log::trace!("Flushing queue: {}", self.queue.len());

        let docs: Vec<Value> = self.queue.drain(0..batch_size).collect();

        log::debug!(
            "Bulk sending {batch_size} docs to {}/{}",
            self.hostname,
            self.index,
        );

        let index = self.index.clone();
        let client: Arc<Elasticsearch> = Arc::clone(&self.client);
        let action = self.action;

        // Spawn a tokio task to send the bulk request
        self.futures.push(tokio::spawn(async move {
            let mut attempt: u64 = 0;
            let mut backoff = Duration::from_secs(1);
            let max_backoff = Duration::from_secs(30);

            loop {
                attempt += 1;
                let ops = build_bulk_operations(action, docs.clone())?;

                let response = client
                    .bulk(BulkParts::Index(&index))
                    .body(ops)
                    .send()
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
}

fn build_bulk_operations(
    action: BulkAction,
    docs: Vec<Value>,
) -> Result<Vec<BulkOperation<Value>>> {
    docs.into_iter()
        .map(|doc| match action {
            BulkAction::Create => Ok(BulkOperation::create(doc).into()),
            BulkAction::Index => Ok(BulkOperation::index(doc).into()),
            BulkAction::Update => update_operation_from_doc(doc),
        })
        .collect()
}

fn update_operation_from_doc(doc: Value) -> Result<BulkOperation<Value>> {
    let (id, doc) = extract_update_id(doc)?;
    let payload = json!({ "doc": doc });
    Ok(BulkOperation::update(id, payload).into())
}

fn extract_update_id(doc: Value) -> Result<(String, Value)> {
    match doc {
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
    use super::*;
    use bytes::BytesMut;
    use elasticsearch::http::request::Body;

    fn bulk_operation_lines(op: BulkOperation<Value>) -> Vec<Value> {
        let mut bytes = BytesMut::new();
        op.write(&mut bytes).expect("write bulk operation");
        let body = String::from_utf8(bytes.to_vec()).expect("bulk body utf8");
        body.lines()
            .map(|line| serde_json::from_str(line).expect("bulk line json"))
            .collect()
    }

    #[test]
    fn build_bulk_operations_create_uses_create_action() {
        let doc = json!({ "message": "hello" });
        let ops = build_bulk_operations(BulkAction::Create, vec![doc]).expect("ops");
        let lines = bulk_operation_lines(ops[0].clone());
        assert_eq!(lines.len(), 2);
        assert!(lines[0].get("create").is_some());
    }

    #[test]
    fn build_bulk_operations_index_uses_index_action() {
        let doc = json!({ "message": "hello" });
        let ops = build_bulk_operations(BulkAction::Index, vec![doc]).expect("ops");
        let lines = bulk_operation_lines(ops[0].clone());
        assert_eq!(lines.len(), 2);
        assert!(lines[0].get("index").is_some());
    }

    #[test]
    fn build_bulk_operations_update_wraps_doc() {
        let doc = json!({ "_id": "1", "message": "hello" });
        let ops = build_bulk_operations(BulkAction::Update, vec![doc]).expect("ops");
        let lines = bulk_operation_lines(ops[0].clone());
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["update"]["_id"], "1");
        assert_eq!(lines[1], json!({ "doc": { "message": "hello" } }));
    }

    #[test]
    fn build_bulk_operations_update_requires_id() {
        let doc = json!({ "message": "hello" });
        let err = build_bulk_operations(BulkAction::Update, vec![doc])
            .err()
            .expect("expected error");
        assert!(err.to_string().contains("_id"));
    }
}

impl Sender for ElasticsearchOutput {
    async fn send(&mut self, value: &Value) -> Result<usize> {
        self.queue.push(value.clone());
        if self.queue.len() >= BATCH_SIZE {
            self.flush().await?
        }
        Ok(0)
    }

    async fn close(mut self) -> Result<usize> {
        self.flush().await?;
        let doc_count = join_all(self.futures)
            .await
            .into_iter()
            .filter_map(Result::ok)
            .filter_map(|result| match result {
                Ok(count) => Some(count),
                Err(e) => {
                    log::error!("{}", e);
                    None
                }
            })
            .sum();

        Ok(doc_count)
    }
}

impl std::fmt::Display for ElasticsearchOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{}", self.hostname, self.index)
    }
}
