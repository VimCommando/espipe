use serde::Deserialize;
use std::{collections::BTreeMap, fmt::Write as _};

const MAX_ERROR_SUMMARIES: usize = 5;
const MAX_ERROR_SUMMARY_LENGTH: usize = 1_024;
const MAX_ERROR_DETAIL_LENGTH: usize = 240;

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
        let mut error_types: BTreeMap<String, u64> = BTreeMap::new();
        if let (Some(true), Some(items)) = (self.errors, &self.items) {
            for item in items {
                if let Some(e) = item.error_message() {
                    *error_types.entry(e).or_insert(0) += 1;
                }
            }
        }

        let mut summaries = error_types.into_iter().collect::<Vec<_>>();
        summaries.sort_by(|(left, left_count), (right, right_count)| {
            right_count.cmp(left_count).then_with(|| left.cmp(right))
        });

        let mut summary = String::new();
        let mut included = 0;
        for (message, count) in &summaries {
            if included == MAX_ERROR_SUMMARIES {
                break;
            }
            let entry = format!(
                "({count}) {}",
                truncate_error_detail(message, MAX_ERROR_DETAIL_LENGTH)
            );
            let separator = if summary.is_empty() { "" } else { ", " };
            if summary.len() + separator.len() + entry.len() > MAX_ERROR_SUMMARY_LENGTH {
                break;
            }
            summary.push_str(separator);
            summary.push_str(&entry);
            included += 1;
        }

        let omitted = summaries.len().saturating_sub(included);
        if omitted > 0 {
            let suffix = format!("; {omitted} additional error summaries omitted");
            let available = MAX_ERROR_SUMMARY_LENGTH.saturating_sub(suffix.len());
            if summary.len() > available {
                summary = truncate_error_detail(&summary, available);
            }
            let _ = write!(summary, "{suffix}");
        }

        summary
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
    r#type: Option<String>,
    reason: Option<String>,
}

impl std::fmt::Display for ErrorCause {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match (&self.r#type, &self.reason) {
            (Some(error_type), Some(reason)) => write!(f, "{error_type} - {reason}"),
            (Some(error_type), None) => write!(f, "{error_type}"),
            (None, Some(reason)) => write!(f, "{reason}"),
            (None, None) => write!(f, "unknown"),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BulkAction {
    Create { create: BulkResponseItem },
    Index { index: BulkResponseItem },
    Update { update: BulkResponseItem },
}

impl BulkAction {
    fn is_success(&self) -> bool {
        match self {
            BulkAction::Create { create } => create.status == 201,
            BulkAction::Index { index } => index.status == 200 || index.status == 201,
            BulkAction::Update { update } => update.status == 200 || update.status == 201,
        }
    }

    fn error_type(&self) -> Option<String> {
        match self {
            BulkAction::Create { create } => create.error.as_ref().map(|e| e.to_string()),
            BulkAction::Index { index } => index.error.as_ref().map(|e| e.to_string()),
            BulkAction::Update { update } => update.error.as_ref().map(|e| e.to_string()),
        }
    }

    fn index(&self) -> String {
        match self {
            BulkAction::Create { create } => create._index.clone(),
            BulkAction::Index { index } => index._index.clone(),
            BulkAction::Update { update } => update._index.clone(),
        }
    }

    fn error_message(&self) -> Option<String> {
        self.error_type()
            .map(|e| normalize_error_detail(&format!("<{}> {e}", self.index())))
    }
}

fn normalize_error_detail(detail: &str) -> String {
    let mut normalized = detail.to_string();
    let mut search_from = 0;
    while let Some(relative_start) = normalized[search_from..].find(" at ") {
        let start = search_from + relative_start;
        let coordinate_start = start + " at ".len();
        let Some(coordinate_length) = coordinate_length(&normalized[coordinate_start..]) else {
            search_from = coordinate_start;
            continue;
        };
        normalized.replace_range(
            coordinate_start..coordinate_start + coordinate_length,
            "<position>",
        );
        search_from = coordinate_start + "<position>".len();
    }
    normalized
}

fn coordinate_length(value: &str) -> Option<usize> {
    let colon = value.find(':')?;
    if colon == 0 || !value[..colon].bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    let column_length = value[colon + 1..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    (column_length > 0).then_some(colon + 1 + column_length)
}

fn truncate_error_detail(detail: &str, max_length: usize) -> String {
    if detail.len() <= max_length {
        return detail.to_string();
    }

    let suffix = "…";
    let end = max_length.saturating_sub(suffix.len());
    let end = (0..=end)
        .rev()
        .find(|index| detail.is_char_boundary(*index))
        .unwrap_or(0);
    format!("{}{}", &detail[..end], suffix)
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
    r#type: Option<String>,
    reason: Option<String>,
    caused_by: Option<CausedBy>,
}

#[derive(Deserialize)]
struct CausedBy {
    r#type: String,
    reason: String,
}

impl std::fmt::Display for ResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if let Some(caused_by) = &self.caused_by {
            return write!(f, "{} - {}", caused_by.r#type, caused_by.reason);
        }

        match (&self.r#type, &self.reason) {
            (Some(error_type), Some(reason)) => write!(f, "{error_type} - {reason}"),
            (Some(error_type), None) => write!(f, "{error_type}"),
            (None, Some(reason)) => write!(f, "{reason}"),
            (None, None) => write!(f, "unknown bulk item error"),
        }
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

#[cfg(test)]
mod tests {
    use super::{BulkResponse, MAX_ERROR_SUMMARY_LENGTH};
    use serde_json::json;

    #[test]
    fn update_noop_counts_as_success() {
        let response = BulkResponse::try_from(json!({
            "errors": false,
            "items": [{
                "update": {
                    "_index": "documents",
                    "_id": "document-1",
                    "status": 200,
                    "result": "noop"
                }
            }]
        }))
        .unwrap();

        assert!(!response.has_errors());
        assert_eq!(response.success_count(), 1);
    }

    #[test]
    fn update_upsert_create_counts_as_success() {
        let response = BulkResponse::try_from(json!({
            "errors": false,
            "items": [{
                "update": {
                    "_index": "documents",
                    "_id": "document-1",
                    "status": 201,
                    "result": "created"
                }
            }]
        }))
        .unwrap();

        assert!(!response.has_errors());
        assert_eq!(response.success_count(), 1);
    }

    #[test]
    fn update_item_errors_are_counted_as_failures() {
        let response = BulkResponse::try_from(json!({
            "errors": true,
            "items": [{
                "update": {
                    "_index": "documents",
                    "_id": "document-1",
                    "status": 409,
                    "error": {
                        "caused_by": {
                            "type": "version_conflict_engine_exception",
                            "reason": "conflict"
                        }
                    }
                }
            }]
        }))
        .unwrap();

        assert!(response.has_errors());
        assert_eq!(response.success_count(), 0);
        assert!(
            response
                .error_counts()
                .contains("version_conflict_engine_exception")
        );
    }

    #[test]
    fn bulk_item_errors_without_caused_by_are_counted_as_failures() {
        let response = BulkResponse::try_from(json!({
            "errors": true,
            "items": [{
                "index": {
                    "_index": "documents",
                    "_id": "document-1",
                    "status": 400,
                    "error": {
                        "type": "mapper_parsing_exception",
                        "reason": "failed to parse field"
                    }
                }
            }]
        }))
        .unwrap();

        assert!(response.has_errors());
        assert_eq!(response.success_count(), 0);
        assert!(
            response
                .error_counts()
                .contains("mapper_parsing_exception - failed to parse field")
        );
    }

    #[test]
    fn error_counts_normalize_dynamic_positions() {
        let items = (0..100)
            .map(|column| {
                json!({
                    "index": {
                        "_index": "docs-content",
                        "_id": column.to_string(),
                        "status": 400,
                        "error": {
                            "type": "illegal_argument_exception",
                            "reason": format!(
                                "Expected text at 1:{column} but found START_OBJECT"
                            )
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        let response = BulkResponse::try_from(json!({"errors": true, "items": items})).unwrap();

        assert_eq!(
            response.error_counts(),
            "(100) <docs-content> illegal_argument_exception - Expected text at <position> but found START_OBJECT"
        );
    }

    #[test]
    fn error_counts_are_bounded_when_many_error_types_are_present() {
        let items = (0..20)
            .map(|index| {
                json!({
                    "index": {
                        "_index": "docs-content",
                        "_id": index.to_string(),
                        "status": 400,
                        "error": {
                            "type": "illegal_argument_exception",
                            "reason": format!("reason-{index}")
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        let response = BulkResponse::try_from(json!({"errors": true, "items": items})).unwrap();
        let summary = response.error_counts();

        assert!(summary.len() <= MAX_ERROR_SUMMARY_LENGTH);
        assert!(summary.contains("additional error summaries omitted"));
    }
}
