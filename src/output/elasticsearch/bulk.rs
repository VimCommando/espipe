use super::{ElasticsearchOutputConfig, bulk_response::BulkResponse};
use crate::{input::InputDocument, output::BulkAction};
use elasticsearch::{
    Elasticsearch,
    http::{
        Method, StatusCode,
        headers::{HeaderMap, HeaderValue},
    },
};
use eyre::{Result, eyre};
use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::{Map, Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle, time::sleep};

#[cfg(test)]
use serde_json::value::RawValue;

pub(super) struct BulkWorker {
    pub(super) client: Elasticsearch,
    pub(super) hostname: String,
    pub(super) index: String,
    pub(super) action: BulkAction,
    pub(super) config: ElasticsearchOutputConfig,
    pub(super) bulk_pipeline: Option<String>,
}

impl BulkWorker {
    pub(super) async fn run(self, mut receiver: mpsc::Receiver<InputDocument>) -> Result<usize> {
        let worker = Arc::new(self);
        let config = worker.config;
        let mut batch = Vec::with_capacity(config.batch_size);
        let mut docs_sent = 0;
        let mut inflight = FuturesUnordered::new();

        while let Some(doc) = receiver.recv().await {
            batch.push(doc);
            if batch.len() >= config.batch_size {
                worker.spawn_flush(&mut inflight, &mut batch)?;
                docs_sent +=
                    reap_inflight_if_needed(&mut inflight, config.max_inflight_requests).await?;
            }
        }

        if !batch.is_empty() {
            worker.spawn_flush(&mut inflight, &mut batch)?;
        }

        while let Some(result) = inflight.next().await {
            docs_sent += result.map_err(eyre::Report::new)??;
        }
        Ok(docs_sent)
    }

    fn spawn_flush(
        self: &Arc<Self>,
        inflight: &mut FuturesUnordered<JoinHandle<Result<usize>>>,
        batch: &mut Vec<InputDocument>,
    ) -> Result<()> {
        let docs = std::mem::replace(batch, Vec::with_capacity(self.config.batch_size));
        let body = build_bulk_body(self.action, &docs)?;
        log::debug!(
            "Bulk sending {} docs to {}/{}",
            docs.len(),
            self.hostname,
            self.index
        );
        let worker = Arc::clone(self);
        inflight.push(tokio::spawn(async move { worker.send_batch(body).await }));
        Ok(())
    }

    async fn send_batch(&self, body: Vec<u8>) -> Result<usize> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            HeaderValue::from_static("application/x-ndjson"),
        );
        let query = self
            .bulk_pipeline
            .as_deref()
            .map(|pipeline| [("pipeline", pipeline)]);
        let path = format!("/{}/_bulk", self.index);
        let mut attempt = 0u64;
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(30);

        loop {
            attempt += 1;
            let response = self
                .client
                .send(
                    Method::Post,
                    &path,
                    headers.clone(),
                    query.as_ref(),
                    Some(body.clone()),
                    None,
                )
                .await?;
            let status = response.status_code();
            let response = response.json::<BulkResponse>().await?;
            match status {
                StatusCode::BAD_REQUEST => {
                    log::error!(
                        "Bulk response: 400 - Bad request ({})",
                        response.error_cause()
                    );
                    return Ok(0);
                }
                StatusCode::TOO_MANY_REQUESTS => {
                    log::warn!(
                        "Bulk response: 429 - Too many requests (attempt {attempt}, backoff {:?}): {}",
                        backoff,
                        response.error_cause()
                    );
                    sleep(backoff).await;
                    backoff = (backoff * 2).min(max_backoff);
                }
                _ => return Ok(count_bulk_successes(status, response)),
            }
        }
    }
}

fn count_bulk_successes(status: StatusCode, response: BulkResponse) -> usize {
    log::debug!("Bulk response status: {status}");
    if response.has_errors() {
        log::warn!(
            "Bulk response contained errors: {}",
            response.error_counts()
        );
    }
    response.success_count()
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
    use super::*;

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
    fn build_bulk_body_wraps_update_and_upsert_documents() {
        for (action, expected_payload) in [
            (BulkAction::Update, json!({"doc":{"a":1}})),
            (
                BulkAction::Upsert,
                json!({"doc":{"a":1}, "doc_as_upsert":true}),
            ),
        ] {
            let docs = vec![document(r#"{"_id":"doc1","a":1}"#)];
            let body = String::from_utf8(build_bulk_body(action, &docs).unwrap()).unwrap();
            let expected_metadata = json!({"update":{"_id":"doc1"}});
            assert_eq!(body, format!("{expected_metadata}\n{expected_payload}\n"));
        }
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
}
