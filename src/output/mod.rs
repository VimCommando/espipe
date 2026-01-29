mod elasticsearch;
mod file;

extern crate elasticsearch as elasticsearch_client;
use crate::client::{Auth, ElasticsearchBuilder, KnownHost};
use elasticsearch::ElasticsearchOutput;
use elasticsearch_client::Elasticsearch;
use eyre::Result;
use file::FileOutput;
use fluent_uri::UriRef;
use serde_json::Value;
use std::path::PathBuf;
use url::Url;

#[derive(Debug)]
pub enum Output {
    Elasticsearch(ElasticsearchOutput),
    File(FileOutput),
    Stdout,
}

impl Output {
    pub fn try_new(
        insecure: bool,
        auth: Auth,
        uri: UriRef<String>,
        request_body_compression: bool,
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
                let output = ElasticsearchOutput::try_new(client, url)?;
                Ok(Output::Elasticsearch(output))
            }
            Some(scheme) if scheme.as_str() == "file" => {
                let path = PathBuf::from(uri.path().as_str());
                let output = FileOutput::try_from(path)?;
                Ok(Output::File(output))
            }
            Some(scheme) => {
                let known_host = KnownHost::try_from(scheme.as_str())?;
                let url = known_host.get_url().join(uri.path().as_str())?;
                let client = Elasticsearch::try_from(known_host)?;
                let output = ElasticsearchOutput::try_new(client, url)?;
                Ok(Output::Elasticsearch(output))
            }
            None => match uri.path().as_str() {
                "-" => Ok(Output::Stdout),
                _ => {
                    let path = PathBuf::from(uri.path().as_str());
                    let output = FileOutput::try_from(path)?;
                    Ok(Output::File(output))
                }
            },
        }
    }

    pub async fn send(&mut self, value: &Value) -> Result<usize> {
        match self {
            Output::Elasticsearch(output) => Ok(output.send(&value).await?),
            Output::File(output) => Ok(output.send(&value).await?),
            Output::Stdout => {
                println!("{value}");
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
    async fn send(&mut self, value: &Value) -> Result<usize>;
    async fn close(self) -> Result<usize>;
}
