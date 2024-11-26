use super::Sender;

use elasticsearch::{BulkOperation, BulkParts, Elasticsearch};
use eyre::{OptionExt, Result};
use futures::{future::join_all, stream::FuturesUnordered};
use serde_json::Value;
use std::sync::Arc;
use tokio::task::JoinHandle;
use url::Url;

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
            queue: Vec::with_capacity(5000),
            futures: FuturesUnordered::new(),
        })
    }

    async fn flush(&mut self) -> Result<()> {
        let count = match self.queue.len() >= 5000 {
            true => 5000,
            false => self.queue.len(),
        };

        let docs = self.queue.drain(0..count);
        let ops: Vec<BulkOperation<serde_json::Value>> = docs
            .into_iter()
            .map(|doc| BulkOperation::create(doc).into())
            .collect();

        log::debug!(
            "Bulk sending {count} docs to {}/{}",
            self.hostname,
            self.index,
        );

        let index = self.index.clone();
        let client: Arc<Elasticsearch> = Arc::clone(&self.client);

        // Spawn a tokio task to send the bulk request
        self.futures.push(tokio::spawn(async move {
            let response = client.bulk(BulkParts::Index(&index)).body(ops).send().await;

            match response {
                Ok(resp) => {
                    let body = resp.text().await;
                    log::trace!("{}", body.unwrap_or_default());
                }
                Err(e) => {
                    log::error!("Failed to send bulk request: {}", e);
                }
            }
            Ok(count)
        }));
        Ok(())
    }
}

impl Sender for ElasticsearchOutput {
    async fn send(&mut self, value: &Value) -> Result<usize> {
        self.queue.push(value.clone());
        log::trace!("Queue size: {}", self.queue.len());
        if self.queue.len() >= 5000 {
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
