mod bulk_response;

use super::Sender;
use bulk_response::BulkResponse;
use elasticsearch::{http::StatusCode, BulkOperation, BulkParts, Elasticsearch};
use eyre::{OptionExt, Result};
use futures::{future::join_all, stream::FuturesUnordered};
use serde_json::Value;
use std::sync::Arc;
use tokio::task::JoinHandle;
use url::Url;

static BATCH_SIZE: usize = 5_000;

#[derive(Debug)]
pub struct ElasticsearchOutput {
    client: Arc<Elasticsearch>,
    hostname: String,
    index: String,
    queue: Vec<Value>,
    futures: FuturesUnordered<JoinHandle<Result<usize>>>,
}

impl ElasticsearchOutput {
    pub fn try_new(client: Elasticsearch, url: Url) -> Result<Self> {
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

        let docs = self.queue.drain(0..batch_size);
        let ops: Vec<BulkOperation<serde_json::Value>> = docs
            .into_iter()
            .map(|doc| BulkOperation::create(doc).into())
            .collect();

        log::debug!(
            "Bulk sending {batch_size} docs to {}/{}",
            self.hostname,
            self.index,
        );

        let index = self.index.clone();
        let client: Arc<Elasticsearch> = Arc::clone(&self.client);

        // Spawn a tokio task to send the bulk request
        self.futures.push(tokio::spawn(async move {
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
                    Ok(0)
                }
                StatusCode::TOO_MANY_REQUESTS => {
                    log::warn!(
                        "Bulk response: 429 - Too many requests ({})",
                        bulk_response.error_cause()
                    );
                    // TODO: Retry the bulk request
                    Ok(0)
                }
                _ => {
                    log::debug!("Bulk response status: {status_code}");
                    if bulk_response.has_errors() {
                        log::warn!(
                            "Bulk response contained errors: {}",
                            bulk_response.error_counts()
                        );
                    }
                    Ok(bulk_response.success_count())
                }
            }
        }));
        Ok(())
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
