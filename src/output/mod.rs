mod action;
mod elasticsearch;
mod file;

extern crate elasticsearch as elasticsearch_client;
use crate::client::{Auth, ElasticsearchBuilder, KnownHost};
pub use action::BulkAction;
use elasticsearch::ElasticsearchOutput;
pub use elasticsearch::{ElasticsearchOutputConfig, TemplateConfig};
use elasticsearch_client::Elasticsearch;
use eyre::Result;
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

impl Output {
    pub async fn try_new(
        insecure: bool,
        auth: Auth,
        uri: UriRef<String>,
        action: BulkAction,
        request_body_compression: bool,
        elasticsearch_config: ElasticsearchOutputConfig,
        template_config: Option<TemplateConfig>,
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
                    template_config,
                )
                .await?;
                Ok(Output::Elasticsearch(output))
            }
            Some(scheme) if scheme.as_str() == "file" => {
                reject_template_config(&template_config)?;
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
                    template_config,
                )
                .await?;
                Ok(Output::Elasticsearch(output))
            }
            None => match uri.path().as_str() {
                "-" => {
                    reject_template_config(&template_config)?;
                    Ok(Output::Stdout)
                }
                _ => {
                    reject_template_config(&template_config)?;
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

fn reject_template_config(template_config: &Option<TemplateConfig>) -> Result<()> {
    if template_config.is_some() {
        return Err(eyre::eyre!(
            "template options require an Elasticsearch output"
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
