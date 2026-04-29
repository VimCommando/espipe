mod action;
mod elasticsearch;
mod file;

extern crate elasticsearch as elasticsearch_client;
use crate::client::{Auth, ElasticsearchBuilder, KnownHost};
pub use action::BulkAction;
use elasticsearch::ElasticsearchOutput;
pub use elasticsearch::ElasticsearchOutputConfig;
use elasticsearch_client::Elasticsearch;
use eyre::{Result, eyre};
use file::FileOutput;
use fluent_uri::UriRef;
use serde_json::value::RawValue;
use std::path::PathBuf;
use url::Url;

#[derive(Debug)]
pub enum Output {
    Elasticsearch(ElasticsearchOutput),
    File(FileOutput),
    Stdout,
}

#[derive(Debug, Default)]
pub struct OutputPreflightConfig {
    pub pipeline: Option<PathBuf>,
    pub pipeline_name: Option<String>,
    pub template: Option<PathBuf>,
    pub template_name: Option<String>,
    pub template_overwrite: Option<bool>,
}

impl OutputPreflightConfig {
    pub fn validate(&self) -> Result<()> {
        if self.template.is_none() {
            if self.template_name.is_some() {
                return Err(eyre!("--template-name requires --template"));
            }
            if self.template_overwrite.is_some() {
                return Err(eyre!("--template-overwrite requires --template"));
            }
        }
        if self.pipeline.is_none()
            && self
                .pipeline_name
                .as_deref()
                .is_some_and(|name| name != "_none")
        {
            return Err(eyre!(
                "--pipeline-name requires --pipeline unless the name is _none"
            ));
        }
        if self.template.is_some()
            && self.pipeline.is_none()
            && self.pipeline_name.as_deref() == Some("_none")
        {
            return Err(eyre!(
                "--pipeline-name _none cannot be used with --template because template-driven bulk requests do not set a request-level pipeline"
            ));
        }
        Ok(())
    }

    pub fn has_elasticsearch_options(&self) -> bool {
        self.pipeline.is_some()
            || self.pipeline_name.is_some()
            || self.template.is_some()
            || self.template_name.is_some()
            || self.template_overwrite.is_some()
    }

    fn has_pipeline_options(&self) -> bool {
        self.pipeline.is_some() || self.pipeline_name.is_some()
    }

    fn has_template_options(&self) -> bool {
        self.template.is_some() || self.template_name.is_some() || self.template_overwrite.is_some()
    }
}

impl Output {
    pub async fn try_new(
        insecure: bool,
        auth: Auth,
        uri: UriRef<String>,
        action: BulkAction,
        request_body_compression: bool,
        elasticsearch_config: ElasticsearchOutputConfig,
        preflight: OutputPreflightConfig,
    ) -> Result<Self> {
        log::trace!("{uri:?}");
        match uri.scheme() {
            Some(scheme) if ["http", "https"].contains(&scheme.as_str()) => {
                let url = Url::parse(uri.as_str())?;
                let mut client_url = url.clone();
                client_url.set_path("");
                let client = ElasticsearchBuilder::new(client_url)
                    .insecure(insecure)
                    .auth(auth)
                    .request_body_compression(request_body_compression)
                    .build()?;
                let output = ElasticsearchOutput::try_new(
                    client,
                    url,
                    action,
                    elasticsearch_config,
                    preflight,
                )
                .await?;
                Ok(Output::Elasticsearch(output))
            }
            Some(scheme) if scheme.as_str() == "file" => {
                reject_elasticsearch_options(&preflight)?;
                let path = PathBuf::from(uri.path().as_str());
                let output = FileOutput::try_from(path)?;
                Ok(Output::File(output))
            }
            Some(scheme) => {
                let known_host = KnownHost::try_from(scheme.as_str())?;
                let url = known_host.get_url().join(uri.path().as_str())?;
                let client = Elasticsearch::try_from(known_host)?;
                let output = ElasticsearchOutput::try_new(
                    client,
                    url,
                    action,
                    elasticsearch_config,
                    preflight,
                )
                .await?;
                Ok(Output::Elasticsearch(output))
            }
            None => match uri.path().as_str() {
                "-" => {
                    reject_elasticsearch_options(&preflight)?;
                    Ok(Output::Stdout)
                }
                _ => {
                    reject_elasticsearch_options(&preflight)?;
                    let path = PathBuf::from(uri.path().as_str());
                    let output = FileOutput::try_from(path)?;
                    Ok(Output::File(output))
                }
            },
        }
    }

    pub async fn send(&mut self, value: Box<RawValue>) -> Result<usize> {
        match self {
            Output::Elasticsearch(output) => Ok(output.send(value).await?),
            Output::File(output) => Ok(output.send(value).await?),
            Output::Stdout => {
                println!("{}", value.get());
                Ok(1)
            }
        }
    }

    pub async fn close(self) -> Result<usize> {
        match self {
            Output::Elasticsearch(output) => Ok(output.close().await?),
            Output::File(output) => Ok(output.close().await?),
            Output::Stdout => Ok(0),
        }
    }
}

fn reject_elasticsearch_options(preflight: &OutputPreflightConfig) -> Result<()> {
    if preflight.has_elasticsearch_options() {
        if preflight.has_template_options() && !preflight.has_pipeline_options() {
            return Err(eyre!("template options require an Elasticsearch output"));
        }
        if preflight.has_pipeline_options() && !preflight.has_template_options() {
            return Err(eyre!("pipeline options require an Elasticsearch output"));
        }
        return Err(eyre!(
            "--pipeline, --pipeline-name, --template, --template-name, and --template-overwrite require an Elasticsearch output"
        ));
    }
    Ok(())
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Output::Elasticsearch(output) => write!(f, "{output}"),
            Output::File(output) => write!(f, "{output}"),
            Output::Stdout => write!(f, "stdout"),
        }
    }
}

trait Sender {
    async fn send(&mut self, value: Box<RawValue>) -> Result<usize>;
    async fn close(self) -> Result<usize>;
}
