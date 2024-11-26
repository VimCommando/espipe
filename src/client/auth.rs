use eyre::{eyre, Result};

pub enum Auth {
    Apikey(String),
    Basic(String, String),
    None,
}

impl Auth {
    pub fn try_new(
        apikey: Option<String>,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<Self> {
        match (apikey, username, password) {
            (Some(apikey), None, None) => Ok(Self::Apikey(apikey)),
            (None, Some(username), Some(password)) => Ok(Self::Basic(username, password)),
            (None, None, None) => Ok(Self::None),
            _ => Err(eyre!("Invalid auth configuration")),
        }
    }
}

impl std::fmt::Display for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Apikey(_) => write!(f, "Apikey"),
            Self::Basic(_, _) => write!(f, "Basic"),
            Self::None => write!(f, "None"),
        }
    }
}
