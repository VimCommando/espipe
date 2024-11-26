use super::auth::Auth;
use super::known_host::KnownHost;
use base64::{engine::general_purpose::STANDARD, Engine};
use elasticsearch::{
    self,
    cert::CertificateValidation,
    http::{
        self,
        transport::{SingleNodeConnectionPool, TransportBuilder},
    },
    Elasticsearch,
};
use eyre::Result;
use url::Url;

pub struct ElasticsearchBuilder {
    cert_validation: CertificateValidation,
    connection_pool: SingleNodeConnectionPool,
    headers: http::headers::HeaderMap,
}

impl ElasticsearchBuilder {
    pub fn new(url: Url) -> Self {
        let mut headers = http::headers::HeaderMap::new();
        headers.append(http::headers::ACCEPT_ENCODING, "gzip".parse().unwrap());

        Self {
            cert_validation: CertificateValidation::Default,
            connection_pool: SingleNodeConnectionPool::new(url),
            headers,
        }
    }

    pub fn insecure(self, ignore_certs: bool) -> Self {
        let cert_validation = match ignore_certs {
            true => CertificateValidation::None,
            false => CertificateValidation::Default,
        };
        Self {
            cert_validation,
            ..self
        }
    }

    pub fn apikey(self, apikey: String) -> Self {
        let mut headers = self.headers;
        headers.append(
            http::headers::AUTHORIZATION,
            format!("ApiKey {}", apikey)
                .parse()
                .expect("Invalid API key"),
        );
        Self { headers, ..self }
    }

    pub fn auth(self, auth: Auth) -> Self {
        log::debug!("Setting client auth to {}", auth);
        match auth {
            Auth::Apikey(apikey) => self.apikey(apikey),
            Auth::Basic(username, password) => self.basic_auth(username, password),
            Auth::None => self,
        }
    }

    pub fn basic_auth(self, username: String, password: String) -> Self {
        let mut headers = self.headers;
        headers.append(
            http::headers::AUTHORIZATION,
            http::headers::HeaderValue::from_str(&format!(
                "Basic {}",
                STANDARD.encode(&format!("{}:{}", username, password))
            ))
            .expect("Invalid basic auth"),
        );
        Self { headers, ..self }
    }

    pub fn build(self) -> Result<elasticsearch::Elasticsearch> {
        let transport = TransportBuilder::new(self.connection_pool)
            .headers(self.headers)
            .cert_validation(self.cert_validation)
            .build()?;
        Ok(elasticsearch::Elasticsearch::new(transport))
    }
}

impl TryFrom<KnownHost> for Elasticsearch {
    type Error = eyre::Report;

    fn try_from(host: KnownHost) -> std::result::Result<Elasticsearch, Self::Error> {
        let client = match host {
            KnownHost::ApiKey {
                apikey,
                url,
                insecure,
            } => ElasticsearchBuilder::new(url)
                .apikey(apikey)
                .insecure(insecure.unwrap_or(false))
                .build()?,
            KnownHost::Basic {
                insecure,
                username,
                password,
                url,
            } => ElasticsearchBuilder::new(url)
                .basic_auth(username, password)
                .insecure(insecure.unwrap_or(false))
                .build()?,
            KnownHost::None { url, insecure } => ElasticsearchBuilder::new(url)
                .insecure(insecure.unwrap_or(false))
                .build()?,
        };
        Ok(client)
    }
}
