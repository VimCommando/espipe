mod bulk;
mod bulk_response;
mod embedded_templates;
mod preflight;
mod template;

use super::{BulkAction, OutputPreflightConfig, Sender};
use crate::input::InputDocument;
use bulk::BulkWorker;
use elasticsearch::Elasticsearch;
use eyre::{OptionExt, Result, eyre};
use preflight::PreparedPreflight;
use std::path::Path;
pub(super) use template::validate_bundled_template;
use tokio::{sync::mpsc, task::JoinHandle};
use url::Url;

pub(crate) struct ElasticsearchOptions {
    pub(crate) action: BulkAction,
    pub(crate) config: ElasticsearchOutputConfig,
    pub(crate) preflight: OutputPreflightConfig,
}

const DEFAULT_BATCH_SIZE: usize = 5_000;
const MULTI_SOURCE_DEFAULT_BATCH_SIZE: usize = 500;
const DEFAULT_MAX_INFLIGHT_REQUESTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ElasticsearchOutputConfig {
    batch_size: usize,
    max_inflight_requests: usize,
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
        options: ElasticsearchOptions,
    ) -> Result<Self> {
        let ElasticsearchOptions {
            action,
            config,
            preflight,
        } = options;
        let hostname = url
            .host_str()
            .ok_or_eyre("Url missing host_str")?
            .to_string();
        let index = url.path().trim_start_matches('/').to_string();
        log::debug!("Elasticsearch output to {hostname}/{index}");

        let preflight = PreparedPreflight::try_from(preflight)?;
        preflight.run(&client, &index).await?;

        let (sender, receiver) = mpsc::channel(config.channel_capacity());
        let worker = BulkWorker {
            client,
            hostname: hostname.clone(),
            index: index.clone(),
            action,
            config,
            bulk_pipeline: preflight.bulk_pipeline,
        };
        let worker = tokio::spawn(worker.run(receiver));

        Ok(Self {
            hostname,
            index,
            sender: Some(sender),
            worker,
        })
    }
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

fn normalized_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

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
