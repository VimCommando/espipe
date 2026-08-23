mod action;
mod elasticsearch;
mod file;

extern crate elasticsearch as elasticsearch_client;
use crate::client::{Auth, ElasticsearchBuilder, KnownHost};
use crate::input::InputDocument;
pub use action::BulkAction;
use elasticsearch::ElasticsearchOutput;
pub use elasticsearch::ElasticsearchOutputConfig;
use elasticsearch_client::Elasticsearch;
use eyre::{Result, eyre};
use file::FileOutput;
use fluent_uri::UriRef;
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
    pub fn validate_preflight_target(
        uri: &UriRef<String>,
        preflight: &OutputPreflightConfig,
    ) -> Result<()> {
        match uri.scheme().map(|scheme| scheme.as_str()) {
            Some("file") | None => reject_elasticsearch_options(preflight),
            _ => Ok(()),
        }
    }

    pub async fn try_new(
        insecure: bool,
        auth: Auth,
        uri: UriRef<String>,
        elastic_cli_url: Option<String>,
        action: BulkAction,
        request_body_compression: bool,
        elasticsearch_config: ElasticsearchOutputConfig,
        preflight: OutputPreflightConfig,
    ) -> Result<Self> {
        log::trace!("{uri:?}");
        match uri.scheme() {
            Some(scheme) if is_elastic_cli_scheme(scheme.as_str()) => {
                let elastic_cli_url = elastic_cli_url.ok_or_else(|| {
                    eyre!(
                        "{} outputs require ELASTIC_ES_URL",
                        elastic_cli_scheme_display(scheme.as_str())
                    )
                })?;
                let index = elastic_cli_index(&uri)?;
                let url = elastic_cli_output_url(&elastic_cli_url, index)?;
                Self::elasticsearch(
                    insecure,
                    auth,
                    url,
                    action,
                    request_body_compression,
                    elasticsearch_config,
                    preflight,
                )
                .await
            }
            Some(scheme) if ["http", "https"].contains(&scheme.as_str()) => {
                let url = Url::parse(uri.as_str())?;
                Self::elasticsearch(
                    insecure,
                    auth,
                    url,
                    action,
                    request_body_compression,
                    elasticsearch_config,
                    preflight,
                )
                .await
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

    async fn elasticsearch(
        insecure: bool,
        auth: Auth,
        url: Url,
        action: BulkAction,
        request_body_compression: bool,
        elasticsearch_config: ElasticsearchOutputConfig,
        preflight: OutputPreflightConfig,
    ) -> Result<Self> {
        let mut client_url = url.clone();
        client_url.set_path("");
        client_url.set_query(None);
        client_url.set_fragment(None);
        let client = ElasticsearchBuilder::new(client_url)
            .insecure(insecure)
            .auth(auth)
            .request_body_compression(request_body_compression)
            .build()?;
        let output =
            ElasticsearchOutput::try_new(client, url, action, elasticsearch_config, preflight)
                .await?;
        Ok(Self::Elasticsearch(output))
    }

    pub async fn send(&mut self, value: InputDocument) -> Result<usize> {
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

fn elastic_cli_output_url(elastic_cli_url: &str, index: &str) -> Result<Url> {
    let mut url =
        Url::parse(elastic_cli_url).map_err(|err| eyre!("Invalid ELASTIC_ES_URL: {err}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(eyre!(
            "ELASTIC_ES_URL must be an absolute http:// or https:// URL"
        ));
    }

    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}/{index}"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn is_elastic_cli_scheme(scheme: &str) -> bool {
    matches!(scheme, "elasticsearch" | "es")
}

fn elastic_cli_scheme_display(scheme: &str) -> &str {
    match scheme {
        "elasticsearch" => "elasticsearch:/index",
        "es" => "es:/index",
        _ => unreachable!("only Elastic CLI output schemes are passed here"),
    }
}

fn elastic_cli_index(uri: &UriRef<String>) -> Result<&str> {
    let path = uri.path().as_str();
    if uri.authority().is_some() || !path.starts_with('/') || path.len() == 1 {
        return Err(eyre!(
            "Elastic CLI outputs must use `elasticsearch:/index` or `es:/index`"
        ));
    }
    Ok(path.trim_start_matches('/'))
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
    async fn send(&mut self, value: InputDocument) -> Result<usize>;
    async fn close(self) -> Result<usize>;
}

#[cfg(test)]
mod tests {
    use super::{elastic_cli_index, elastic_cli_output_url, is_elastic_cli_scheme};
    use fluent_uri::UriRef;

    #[test]
    fn elastic_cli_url_appends_index_to_base_path() {
        let url = elastic_cli_output_url(
            "https://example.com/elasticsearch/?ignored=true#fragment",
            "logs-2026",
        )
        .unwrap();

        assert_eq!(url.as_str(), "https://example.com/elasticsearch/logs-2026");
    }

    #[test]
    fn elastic_cli_url_requires_an_absolute_http_url() {
        let err = elastic_cli_output_url("file:///tmp/elasticsearch", "logs").unwrap_err();

        assert!(err.to_string().contains("http:// or https://"));
    }

    #[test]
    fn elastic_cli_schemes_are_reserved_for_context_outputs() {
        assert!(is_elastic_cli_scheme("elasticsearch"));
        assert!(is_elastic_cli_scheme("es"));
        assert!(!is_elastic_cli_scheme("production"));
    }

    #[test]
    fn elastic_cli_index_requires_a_single_slash_after_scheme() {
        let es = UriRef::parse("es:/logs-2026".to_string()).unwrap();
        assert_eq!(elastic_cli_index(&es).unwrap(), "logs-2026");

        let missing_slash = UriRef::parse("es:logs-2026".to_string()).unwrap();
        assert!(elastic_cli_index(&missing_slash).is_err());

        let authority = UriRef::parse("es://logs-2026".to_string()).unwrap();
        assert!(elastic_cli_index(&authority).is_err());
    }
}
