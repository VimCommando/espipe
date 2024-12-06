use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct BulkResponse {
    error: Option<ErrorType>,
    //took: u64,
    errors: Option<bool>,
    items: Option<Vec<BulkAction>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ErrorType {
    Object(ErrorCause),
    String(String),
}

impl std::fmt::Display for ErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ErrorType::Object(e) => write!(f, "{}", e),
            ErrorType::String(s) => write!(f, "{}", s),
        }
    }
}

impl BulkResponse {
    pub fn error_cause(&self) -> String {
        match &self.error {
            Some(cause) => format!("{cause}"),
            None => "unknown".to_string(),
        }
    }

    pub fn error_counts(&self) -> String {
        let mut error_types: HashMap<String, u64> = HashMap::new();
        if let (Some(true), Some(items)) = (self.errors, &self.items) {
            items.iter().for_each(|item| {
                item.error_message().map(|e| {
                    *error_types.entry(e).or_insert(0) += 1;
                });
            })
        }

        error_types
            .into_iter()
            .map(|(k, v)| format!("({v}) {k}"))
            .collect::<Vec<String>>()
            .join(", ")
    }

    pub fn has_errors(&self) -> bool {
        match self.errors {
            Some(true) => true,
            _ => false,
        }
    }

    pub fn success_count(&self) -> usize {
        match &self.items {
            Some(items) => items.iter().filter(|item| item.is_success()).count(),
            None => 0,
        }
    }
}

#[derive(Deserialize)]
struct ErrorCause {
    r#type: String,
    //reason: String,
}

impl std::fmt::Display for ErrorCause {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.r#type)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BulkAction {
    Create { create: BulkResponseItem },
    Index { index: BulkResponseItem },
}

impl BulkAction {
    fn is_success(&self) -> bool {
        match self {
            BulkAction::Create { create } => create.status == 201,
            BulkAction::Index { index } => index.status == 200 || index.status == 201,
        }
    }

    fn error_type(&self) -> Option<String> {
        match self {
            BulkAction::Create { create } => create.error.as_ref().map(|e| e.to_string()),
            BulkAction::Index { index } => index.error.as_ref().map(|e| e.to_string()),
        }
    }

    fn index(&self) -> String {
        match self {
            BulkAction::Create { create } => create._index.clone(),
            BulkAction::Index { index } => index._index.clone(),
        }
    }

    fn error_message(&self) -> Option<String> {
        self.error_type().map(|e| format!("<{}> {e}", self.index()))
    }
}

#[derive(Deserialize)]
struct BulkResponseItem {
    _index: String,
    _id: String,
    status: u16,
    error: Option<ResponseError>,
}

#[derive(Deserialize)]
struct ResponseError {
    //r#type: String,
    //reason: String,
    caused_by: CausedBy,
}

#[derive(Deserialize)]
struct CausedBy {
    r#type: String,
    reason: String,
}

impl std::fmt::Display for ResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{} - {}", self.caused_by.r#type, self.caused_by.reason)
    }
}

impl TryFrom<serde_json::Value> for BulkResponse {
    type Error = eyre::Report;
    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        let response: BulkResponse = serde_json::from_value(value)
            .map_err(|e| eyre::eyre!("Failed to parse BulkResponse: {:?}", e))?;
        Ok(response)
    }
}
