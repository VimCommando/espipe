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
pub enum OutputTarget {
    Context(ElasticContextOutputTarget),
    Uri(UriRef<String>),
}

#[derive(Debug)]
pub(crate) struct ElasticContextOutputTarget {
    reference: elasticrc::ContextServiceReference,
    index: String,
}

impl OutputTarget {
    pub fn parse(value: String) -> Result<Self> {
        if let Some(target) = ElasticContextOutputTarget::parse(&value)? {
            return Ok(Self::Context(target));
        }
        let uri = UriRef::parse(value)
            .map_err(|(err, value)| eyre!("invalid output URI '{value}': {err}"))?;
        Ok(Self::Uri(uri))
    }

    pub fn is_file_output(&self) -> bool {
        match self {
            Self::Context(_) => false,
            Self::Uri(uri) => match uri.scheme().map(|scheme| scheme.as_str()) {
                Some("file") => true,
                None => uri.path().as_str() != "-",
                _ => false,
            },
        }
    }

    pub fn file_path(&self) -> Option<&str> {
        match self {
            Self::Context(_) => None,
            Self::Uri(uri) if self.is_file_output() => Some(uri.path().as_str()),
            Self::Uri(_) => None,
        }
    }
}

impl ElasticContextOutputTarget {
    fn parse(value: &str) -> Result<Option<Self>> {
        let Some((reference_value, index_path)) = value.split_once(':') else {
            return Ok(None);
        };
        if !reference_value.starts_with('.') {
            return Ok(None);
        }
        let expected_form = "Elastic CLI context outputs must use `.context.app:/index`";
        let parsed_reference = elasticrc::ContextServiceReference::parse(reference_value);
        if !index_path.starts_with('/') {
            return match parsed_reference {
                Some(_) => Err(eyre!(expected_form)),
                None => Ok(None),
            };
        }
        if index_path.len() == 1 || index_path.starts_with("//") {
            return Err(eyre!(expected_form));
        }
        let reference = parsed_reference.ok_or_else(|| {
            let application = reference_value
                .rsplit('.')
                .next()
                .filter(|value| !value.is_empty())
                .unwrap_or(reference_value);
            eyre!("unsupported Elastic CLI context application '{application}'")
        })?;
        if reference.service != elasticrc::ServiceKind::Elasticsearch {
            return Err(eyre!(
                "Elastic CLI context outputs must select Elasticsearch, not {}",
                reference.service
            ));
        }
        Ok(Some(Self {
            reference,
            index: index_path.trim_start_matches('/').to_string(),
        }))
    }

    fn resolve(self) -> Result<(Url, elasticrc::ResolvedAuth)> {
        let config = elasticrc::ConfigFile::load_with_options(None, None)
            .map_err(|err| eyre!("Could not load Elastic CLI config: {err}"))?;
        let service = match self.reference.context.as_deref() {
            Some(context) => config
                .resolve_service(context, elasticrc::ServiceKind::Elasticsearch)
                .map_err(|err| eyre!("Could not resolve Elastic CLI context '{context}': {err}"))?,
            None => config
                .resolve_current_service(elasticrc::ServiceKind::Elasticsearch)
                .map_err(|err| eyre!("Could not resolve the active Elastic CLI context: {err}"))?,
        };
        Ok((context_output_url(service.url, &self.index), service.auth))
    }
}

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
    pub fn validate_environment_target(target: &OutputTarget) -> Result<bool> {
        let OutputTarget::Uri(uri) = target else {
            return Ok(false);
        };
        if uri
            .scheme()
            .is_some_and(|scheme| is_env_scheme(scheme.as_str()))
        {
            env_index(uri)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn validate_preflight_target(
        target: &OutputTarget,
        preflight: &OutputPreflightConfig,
    ) -> Result<()> {
        let OutputTarget::Uri(uri) = target else {
            if let Some(template) = &preflight.template {
                elasticsearch::validate_bundled_template(template)?;
            }
            return Ok(());
        };
        match uri.scheme().map(|scheme| scheme.as_str()) {
            Some("file") | None => reject_elasticsearch_options(preflight),
            _ => {
                if let Some(template) = &preflight.template {
                    elasticsearch::validate_bundled_template(template)?;
                }
                Ok(())
            }
        }
    }

    pub async fn try_new(
        insecure: bool,
        auth: Auth,
        target: OutputTarget,
        environment_url: Option<String>,
        action: BulkAction,
        request_body_compression: bool,
        elasticsearch_config: ElasticsearchOutputConfig,
        preflight: OutputPreflightConfig,
    ) -> Result<Self> {
        log::trace!("{target:?}");
        let uri = match target {
            OutputTarget::Context(target) => {
                let (url, context_auth) = target.resolve()?;
                let auth = match auth {
                    Auth::None => auth_from_context(context_auth),
                    explicit => explicit,
                };
                return Self::elasticsearch(
                    insecure,
                    auth,
                    url,
                    action,
                    request_body_compression,
                    elasticsearch_config,
                    preflight,
                )
                .await;
            }
            OutputTarget::Uri(uri) => uri,
        };
        match uri.scheme() {
            Some(scheme) if is_env_scheme(scheme.as_str()) => {
                let environment_url = environment_url
                    .ok_or_else(|| eyre!("env:/index outputs require ELASTIC_ES_URL"))?;
                let index = env_index(&uri)?;
                let url = environment_output_url(&environment_url, index)?;
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

fn auth_from_context(auth: elasticrc::ResolvedAuth) -> Auth {
    match auth {
        elasticrc::ResolvedAuth::ApiKey(api_key) => Auth::Apikey(api_key.expose_secret().clone()),
        elasticrc::ResolvedAuth::Basic { username, password } => {
            Auth::Basic(username, password.expose_secret().clone())
        }
        elasticrc::ResolvedAuth::None => Auth::None,
    }
}

fn context_output_url(mut url: Url, index: &str) -> Url {
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}/{index}"));
    url.set_query(None);
    url.set_fragment(None);
    url
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

fn environment_output_url(environment_url: &str, index: &str) -> Result<Url> {
    let mut url =
        Url::parse(environment_url).map_err(|err| eyre!("Invalid ELASTIC_ES_URL: {err}"))?;
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

fn is_env_scheme(scheme: &str) -> bool {
    scheme == "env"
}

fn env_index(uri: &UriRef<String>) -> Result<&str> {
    let path = uri.path().as_str();
    if uri.authority().is_some() || !path.starts_with('/') || path.len() == 1 {
        return Err(eyre!("environment outputs must use `env:/index`"));
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
    use super::{Output, OutputTarget, env_index, environment_output_url, is_env_scheme};
    use fluent_uri::UriRef;

    #[test]
    fn environment_url_appends_index_to_base_path() {
        let url = environment_output_url(
            "https://example.com/elasticsearch/?ignored=true#fragment",
            "logs-2026",
        )
        .unwrap();

        assert_eq!(url.as_str(), "https://example.com/elasticsearch/logs-2026");
    }

    #[test]
    fn environment_url_requires_an_absolute_http_url() {
        let err = environment_output_url("file:///tmp/elasticsearch", "logs").unwrap_err();

        assert!(err.to_string().contains("http:// or https://"));
    }

    #[test]
    fn env_scheme_is_reserved_for_environment_outputs() {
        assert!(is_env_scheme("env"));
        assert!(!is_env_scheme("elasticsearch"));
        assert!(!is_env_scheme("es"));
        assert!(!is_env_scheme("production"));
    }

    #[test]
    fn env_index_requires_a_single_slash_after_scheme() {
        let env = UriRef::parse("env:/logs-2026".to_string()).unwrap();
        assert_eq!(env_index(&env).unwrap(), "logs-2026");

        let missing_slash = UriRef::parse("env:logs-2026".to_string()).unwrap();
        assert!(env_index(&missing_slash).is_err());

        let authority = UriRef::parse("env://logs-2026".to_string()).unwrap();
        assert!(env_index(&authority).is_err());
    }

    #[test]
    fn environment_target_validation_rejects_invalid_uri_forms() {
        let valid = OutputTarget::parse("env:/logs-2026".to_string()).unwrap();
        assert!(Output::validate_environment_target(&valid).unwrap());

        for invalid in ["env:/", "env:logs-2026", "env://logs-2026"] {
            let target = OutputTarget::parse(invalid.to_string()).unwrap();
            assert!(Output::validate_environment_target(&target).is_err());
        }

        let direct = OutputTarget::parse("https://example.com/logs".to_string()).unwrap();
        assert!(!Output::validate_environment_target(&direct).unwrap());
    }
}
