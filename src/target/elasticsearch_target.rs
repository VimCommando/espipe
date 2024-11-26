use super::Sender;

use elasticsearch::{BulkOperation, BulkParts, Elasticsearch};
use eyre::{OptionExt, Result};
use serde_json::Value;
use url::Url;

#[derive(Debug)]
pub struct ElasticsearchOutput {
    client: Elasticsearch,
    hostname: String,
    index: String,
    queue: Vec<Value>,
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
            client,
            hostname,
            index,
            queue: Vec::with_capacity(5000),
        })
    }

    async fn flush(&mut self) -> Result<usize> {
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
            "Bulk sending {count} docs to {}, {}",
            &self.hostname,
            &self.index,
        );
        let response = self
            .client
            .bulk(BulkParts::Index(&self.index))
            .body(ops)
            .send()
            .await?;

        let body = response.text().await?;
        log::trace!("{}", body);
        Ok(count)
    }
}

impl Sender for ElasticsearchOutput {
    async fn send(&mut self, value: &Value) -> Result<usize> {
        self.queue.push(value.clone());
        log::trace!("Queue size: {}", self.queue.len());
        let doc_count = if self.queue.len() >= 5000 {
            self.flush().await?
        } else {
            0
        };
        Ok(doc_count)
    }

    async fn close(&mut self) -> Result<usize> {
        self.flush().await
    }
}

impl std::fmt::Display for ElasticsearchOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.hostname)
    }
}
