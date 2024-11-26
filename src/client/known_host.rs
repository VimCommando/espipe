use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use serde_yaml;
use std::collections::BTreeMap;
use std::env;
use std::fmt::{self, Display, Formatter};
use std::fs::{create_dir, File};
use std::io::BufReader;
use std::path::PathBuf;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "auth")]
pub enum KnownHost {
    ApiKey {
        insecure: Option<bool>,
        apikey: String,
        url: Url,
    },
    Basic {
        insecure: Option<bool>,
        password: String,
        url: Url,
        username: String,
    },
    None {
        insecure: Option<bool>,
        url: Url,
    },
}

impl KnownHost {
    pub fn parse(host: &str) -> Option<Self> {
        // parse the ~/.espipe/hosts.yml file into a HashMap<String, Host>
        let hosts = match parse_hosts_yml() {
            Ok(hosts) => hosts,
            Err(e) => {
                log::error!("Error parsing hosts.yml: {}", e);
                return None;
            }
        };
        log::debug!(
            "Known hosts: {}",
            hosts
                .clone()
                .into_iter()
                .map(|(k, _)| k)
                .collect::<Vec<String>>()
                .join(", ")
        );
        hosts.get(host).cloned()
    }

    pub fn get_url(&self) -> Url {
        match self {
            Self::ApiKey { url, .. } => url.clone(),
            Self::Basic { url, .. } => url.clone(),
            Self::None { url, .. } => url.clone(),
        }
    }
}

impl Display for KnownHost {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiKey { url, .. } => write!(fmt, "ApiKey auth: {}", url,),
            Self::Basic { url, username, .. } => write!(fmt, "Basic auth: {}@ {}", username, url,),
            Self::None { url, .. } => write!(fmt, "No auth: {}", url),
        }
    }
}

impl TryFrom<&str> for KnownHost {
    type Error = eyre::Report;
    fn try_from(value: &str) -> Result<Self> {
        match KnownHost::parse(value) {
            Some(host) => Ok(host),
            None => Err(eyre!("No known host entry for: {}", value)),
        }
    }
}

/// Get the path for the hosts.yml file, fallback to ~/.espipe/hosts.yml
fn get_hosts_path() -> Result<PathBuf> {
    match env::var("ESPIPE_HOSTS") {
        Ok(path) => Ok(PathBuf::from(path)),
        Err(_) => {
            let home = env::var("HOME").map(|home| PathBuf::from(home))?;
            // Check if the `.espipe` directory exists, if not, create it
            let home_dir = home.join(".espipe");
            if !home_dir.exists() {
                create_dir(&home_dir)?
            }
            let path = home.join(".espipe").join("hosts.yml");
            Ok(path)
        }
    }
}

/// Tries to load hosts from a yml file, creates an empty file if it doesn't exist
fn parse_hosts_yml() -> Result<BTreeMap<String, KnownHost>> {
    let path = get_hosts_path()?;
    log::debug!("Parsing {:?}", path);
    match path.is_file() {
        true => {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            let hosts: BTreeMap<String, KnownHost> = serde_yaml::from_reader(reader)?;
            Ok(hosts)
        }
        false => {
            log::info!("No known hosts file, creating: {:?}", path);
            File::create(path)?;
            Ok(BTreeMap::new())
        }
    }
}
