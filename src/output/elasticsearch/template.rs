use super::{embedded_templates, normalized_extension};
use elasticsearch::{
    Elasticsearch,
    http::{
        Method, StatusCode,
        headers::{HeaderMap, HeaderValue},
    },
};
use eyre::{Result, eyre};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(super) struct TemplateConfig {
    source: TemplateSource,
    name: Option<String>,
    overwrite: bool,
}

#[derive(Clone, Debug)]
enum TemplateSource {
    File(PathBuf),
    Bundled(String),
}

impl TemplateConfig {
    pub(super) fn try_new(
        path: Option<PathBuf>,
        name: Option<String>,
        overwrite: Option<bool>,
    ) -> Result<Option<Self>> {
        let path = match (path, name.as_ref(), overwrite) {
            (Some(path), _, _) => path,
            (None, Some(_), _) => return Err(eyre!("--template-name requires --template")),
            (None, _, Some(_)) => return Err(eyre!("--template-overwrite requires --template")),
            (None, None, None) => return Ok(None),
        };
        let source = match path.to_str() {
            Some(selector) if selector.starts_with('_') => {
                TemplateSource::Bundled(selector.to_string())
            }
            _ => TemplateSource::File(path),
        };

        Ok(Some(Self {
            source,
            name,
            overwrite: overwrite.unwrap_or(true),
        }))
    }
}

pub(in crate::output) fn validate_bundled_template(path: &Path) -> Result<()> {
    if let Some(selector) = path.to_str().filter(|value| value.starts_with('_')) {
        embedded_templates::resolve(selector)?;
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct ParsedTemplate {
    pub(super) name: String,
    overwrite: bool,
    pub(super) body: Value,
    bundled: bool,
}

pub(super) async fn install_template(
    client: &Elasticsearch,
    target_index: &str,
    parsed: &ParsedTemplate,
) -> Result<()> {
    if parsed.bundled {
        return install_bundled_template(client, target_index, parsed).await;
    }

    warn_for_index_patterns(&parsed.body, target_index);
    write_template(client, parsed, &parsed.body).await
}

async fn install_bundled_template(
    client: &Elasticsearch,
    target_index: &str,
    parsed: &ParsedTemplate,
) -> Result<()> {
    let path = format!("/_index_template/{}", parsed.name);
    let response = client
        .send(
            Method::Get,
            &path,
            HeaderMap::new(),
            Option::<&()>::None,
            Option::<Vec<u8>>::None,
            None,
        )
        .await
        .map_err(|err| eyre!("failed to look up index template '{}': {err}", parsed.name))?;

    match response.status_code() {
        StatusCode::NOT_FOUND => {
            let mut body = parsed.body.clone();
            append_exact_index(&mut body, target_index).map_err(|err| {
                eyre!(
                    "bundled template '{}' cannot be installed: {err}",
                    parsed.name
                )
            })?;
            write_template(client, parsed, &body).await
        }
        status if status.is_success() => {
            let response_body = response.json::<Value>().await.map_err(|err| {
                eyre!(
                    "failed to parse index template lookup response for '{}': {err}",
                    parsed.name
                )
            })?;
            let mut stored = extract_stored_template(&response_body, &parsed.name)?;
            if !append_exact_index(&mut stored, target_index)? {
                return Ok(());
            }
            if !parsed.overwrite {
                return Err(eyre!(
                    "index template '{}' does not list target index '{target_index}'; appending it requires --template-overwrite=true",
                    parsed.name
                ));
            }
            write_template(client, parsed, &stored).await
        }
        status => {
            let details = response
                .text()
                .await
                .unwrap_or_else(|err| format!("failed to read error body: {err}"));
            Err(eyre!(
                "failed to look up index template '{}': status {status}: {details}",
                parsed.name
            ))
        }
    }
}

fn extract_stored_template(response: &Value, selected_name: &str) -> Result<Value> {
    let entries = response
        .get("index_templates")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            eyre!("index template lookup for '{selected_name}' has no index_templates array")
        })?;
    let matches = entries
        .iter()
        .filter(|entry| entry.get("name").and_then(Value::as_str) == Some(selected_name))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(eyre!(
            "index template lookup for '{selected_name}' returned {} exact matches; expected one",
            matches.len()
        ));
    }
    let body = matches[0]
        .get("index_template")
        .filter(|body| body.is_object())
        .ok_or_else(|| {
            eyre!("index template lookup for '{selected_name}' has no composable template body")
        })?
        .clone();
    validate_index_patterns_array(&body)?;
    Ok(body)
}

fn validate_index_patterns_array(template: &Value) -> Result<()> {
    let patterns = template
        .get("index_patterns")
        .and_then(Value::as_array)
        .ok_or_else(|| eyre!("index_patterns must be an array of strings"))?;
    if patterns.iter().any(|pattern| !pattern.is_string()) {
        return Err(eyre!("index_patterns must be an array of strings"));
    }
    Ok(())
}

fn append_exact_index(template: &mut Value, target_index: &str) -> Result<bool> {
    validate_index_patterns_array(template)?;
    let patterns = template
        .get_mut("index_patterns")
        .and_then(Value::as_array_mut)
        .expect("validated above");
    if patterns
        .iter()
        .any(|pattern| pattern.as_str() == Some(target_index))
    {
        return Ok(false);
    }
    patterns.push(Value::String(target_index.to_string()));
    Ok(true)
}

async fn write_template(
    client: &Elasticsearch,
    parsed: &ParsedTemplate,
    body: &Value,
) -> Result<()> {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let path = format!("/_index_template/{}", parsed.name);
    let method = if parsed.overwrite {
        Method::Put
    } else {
        Method::Post
    };
    let params = if parsed.overwrite {
        None
    } else {
        Some(&[("create", "true")][..])
    };
    let body = serde_json::to_vec(body)?;
    let response = client
        .send(method, &path, headers, params, Some(body), None)
        .await
        .map_err(|err| eyre!("failed to install index template '{}': {err}", parsed.name))?;
    let status = response.status_code();
    if !status.is_success() {
        let details = response
            .text()
            .await
            .unwrap_or_else(|err| format!("failed to read error body: {err}"));
        return Err(eyre!(
            "failed to install index template '{}': status {status}: {details}",
            parsed.name
        ));
    }

    Ok(())
}

pub(super) fn parse_template(config: TemplateConfig) -> Result<ParsedTemplate> {
    let (body, default_name, bundled) = match config.source {
        TemplateSource::File(path) => {
            let contents = std::fs::read_to_string(&path)
                .map_err(|err| eyre!("failed to read template '{}': {err}", path.display()))?;
            let body = parse_config_body("template", &path, &contents)?;
            let name = derive_template_name(&path)?;
            (body, name, false)
        }
        TemplateSource::Bundled(selector) => {
            let embedded = embedded_templates::resolve(&selector)?;
            (embedded.body, embedded.default_name, true)
        }
    };
    let name = match config.name {
        Some(name) => name,
        None => default_name,
    };
    if name.is_empty() {
        return Err(eyre!("template name must be non-empty"));
    }

    Ok(ParsedTemplate {
        name,
        overwrite: config.overwrite,
        body,
        bundled,
    })
}

fn derive_template_name(path: &Path) -> Result<String> {
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| eyre!("template name must be non-empty"))?;
    if name.is_empty() {
        return Err(eyre!("template name must be non-empty"));
    }
    Ok(name.to_string())
}

fn warn_for_index_patterns(template: &Value, target_index: &str) {
    match index_patterns_match(template, target_index) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("warning: template index_patterns do not match target index '{target_index}'")
        }
        Err(reason) => eprintln!(
            "warning: could not verify template index_patterns for target index '{target_index}': {reason}"
        ),
    }
}

fn index_patterns_match(template: &Value, target_index: &str) -> Result<bool> {
    let patterns = template
        .get("index_patterns")
        .ok_or_else(|| eyre!("index_patterns is missing"))?;
    let expressions = match patterns {
        Value::String(pattern) => vec![pattern.as_str()],
        Value::Array(patterns) => patterns
            .iter()
            .map(|pattern| {
                pattern
                    .as_str()
                    .ok_or_else(|| eyre!("index_patterns must contain only strings"))
            })
            .collect::<Result<Vec<_>>>()?,
        _ => return Err(eyre!("index_patterns must be a string or string array")),
    };

    expressions
        .into_iter()
        .flat_map(|expression| expression.split(','))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .try_fold(false, |matched, part| match part.strip_prefix('-') {
            Some("") => Err(eyre!("invalid lone '-' index pattern")),
            Some(pattern) if wildcard_match(pattern, target_index) => Ok(false),
            None if wildcard_match(part, target_index) => Ok(true),
            _ => Ok(matched),
        })
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let (mut pattern_index, mut value_index) = (0usize, 0usize);
    let mut star_index = None;
    let mut star_value_index = 0usize;

    while value_index < value.len() {
        match pattern.get(pattern_index) {
            Some(&byte) if byte == value[value_index] => {
                pattern_index += 1;
                value_index += 1;
            }
            Some(b'*') => {
                star_index = Some(pattern_index);
                star_value_index = value_index;
                pattern_index += 1;
            }
            _ => {
                let Some(star) = star_index else {
                    return false;
                };
                pattern_index = star + 1;
                star_value_index += 1;
                value_index = star_value_index;
            }
        }
    }

    pattern[pattern_index..].iter().all(|&byte| byte == b'*')
}

fn parse_config_body(kind: &str, path: &Path, body: &str) -> Result<Value> {
    match normalized_extension(path).as_deref() {
        Some("jsonc" | "json5") => serde_json5::from_str::<Value>(body)
            .map_err(|err| eyre!("failed to parse {kind} file {}: {err}", path.display())),
        Some("yml" | "yaml") => yaml_serde::from_str::<Value>(body).map_err(|err| {
            eyre!(
                "failed to parse {kind} file {} as YAML: {err}",
                path.display()
            )
        }),
        _ => serde_json::from_str::<Value>(body).map_err(|err| {
            eyre!(
                "failed to parse {kind} file {} as JSON: {err}",
                path.display()
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn template_name_and_overwrite_follow_the_file_and_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs-docs.json");
        std::fs::write(&path, r#"{"index_patterns":["logs-*"]}"#).unwrap();

        for (name, overwrite, expected) in [
            (None, true, "logs-docs"),
            (Some("custom-template"), false, "custom-template"),
        ] {
            let parsed = parse_template(TemplateConfig {
                source: TemplateSource::File(path.clone()),
                name: name.map(str::to_string),
                overwrite,
            })
            .unwrap();
            assert_eq!(
                (parsed.name.as_str(), parsed.overwrite),
                (expected, overwrite)
            );
        }
        let err = parse_template(TemplateConfig {
            source: TemplateSource::File(path),
            name: Some(String::new()),
            overwrite: true,
        })
        .unwrap_err();
        assert!(err.to_string().contains("template name must be non-empty"));
    }

    #[test]
    fn bundled_template_uses_default_and_overridden_names() {
        let default = parse_template(
            TemplateConfig::try_new(Some(PathBuf::from("_okf")), None, None)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(default.name, "open-knowledge-format");
        assert!(default.bundled);

        let overridden = parse_template(
            TemplateConfig::try_new(
                Some(PathBuf::from("_okf")),
                Some("team-okf".to_string()),
                None,
            )
            .unwrap()
            .unwrap(),
        )
        .unwrap();
        assert_eq!(overridden.name, "team-okf");
        assert!(overridden.bundled);
    }

    #[test]
    fn template_paths_with_underscores_outside_the_first_character_stay_files() {
        let config =
            TemplateConfig::try_new(Some(PathBuf::from("templates/_okf.json")), None, None)
                .unwrap()
                .unwrap();
        assert!(matches!(config.source, TemplateSource::File(_)));
    }

    #[test]
    fn stored_template_extraction_requires_one_exact_valid_body() {
        let body = json!({
            "index_templates": [{
                "name": "open-knowledge-format",
                "index_template": {"index_patterns": ["knowledge-a"], "priority": 7}
            }]
        });
        let stored = extract_stored_template(&body, "open-knowledge-format").unwrap();
        assert_eq!(stored["priority"], 7);

        assert!(extract_stored_template(&json!({}), "open-knowledge-format").is_err());
        assert!(
                extract_stored_template(
                    &json!({"index_templates": [{"name": "other", "index_template": {"index_patterns": []}}]}),
                    "open-knowledge-format"
                )
                .is_err()
            );
        assert!(
                extract_stored_template(
                    &json!({"index_templates": [{"name": "open-knowledge-format", "index_template": {"index_patterns": "knowledge-*"}}]}),
                    "open-knowledge-format"
                )
                .is_err()
            );
    }

    #[test]
    fn exact_target_append_ignores_wildcard_coverage() {
        let mut body = json!({"index_patterns": ["team-*"]});
        assert!(append_exact_index(&mut body, "team-knowledge").unwrap());
        assert_eq!(body["index_patterns"], json!(["team-*", "team-knowledge"]));
        assert!(!append_exact_index(&mut body, "team-knowledge").unwrap());
    }

    #[test]
    fn json_and_unknown_extensions_require_strict_json() {
        let dir = tempfile::tempdir().unwrap();
        for filename in ["template.json", "template.txt"] {
            let path = dir.path().join(filename);
            std::fs::write(&path, r#"{"index_patterns":["logs-*"] /* no */}"#).unwrap();
            let config = TemplateConfig {
                source: TemplateSource::File(path.clone()),
                name: None,
                overwrite: true,
            };
            let err = parse_template(config.clone()).unwrap_err();
            assert!(err.to_string().contains(&path.display().to_string()));

            std::fs::write(path, r#"{"index_patterns":["logs-*"]}"#).unwrap();
            let parsed = parse_template(config).unwrap();
            assert_eq!(parsed.body["index_patterns"][0], "logs-*");
        }
    }

    #[test]
    fn jsonc_and_json5_templates_are_normalized() {
        let dir = tempfile::tempdir().unwrap();
        let jsonc_path = dir.path().join("template.jsonc");
        std::fs::write(
            &jsonc_path,
            r#"{"index_patterns":["logs-*"], /* comment */ "priority": 1}"#,
        )
        .unwrap();
        let json5_path = dir.path().join("template.json5");
        std::fs::write(
            &json5_path,
            r#"{index_patterns:["logs-*"], template: { settings: { number_of_shards: 1 } }}"#,
        )
        .unwrap();

        let jsonc = parse_template(TemplateConfig {
            source: TemplateSource::File(jsonc_path),
            name: None,
            overwrite: true,
        })
        .unwrap();
        let json5 = parse_template(TemplateConfig {
            source: TemplateSource::File(json5_path),
            name: None,
            overwrite: true,
        })
        .unwrap();

        assert_eq!(jsonc.body["priority"], 1);
        assert_eq!(json5.body["template"]["settings"]["number_of_shards"], 1);
    }

    #[test]
    fn yaml_template_is_normalized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs-docs.YML");
        std::fs::write(
            &path,
            r#"
    index_patterns:
      - logs-*
    template:
      settings:
        number_of_shards: 1
    "#,
        )
        .unwrap();

        let parsed = parse_template(TemplateConfig {
            source: TemplateSource::File(path),
            name: None,
            overwrite: true,
        })
        .unwrap();

        assert_eq!(parsed.name, "logs-docs");
        assert_eq!(parsed.body["index_patterns"][0], "logs-*");
        assert_eq!(parsed.body["template"]["settings"]["number_of_shards"], 1);
    }

    #[test]
    fn index_patterns_follow_multi_target_ordering() {
        assert!(index_patterns_match(&json!({"index_patterns":"test*"}), "test3").unwrap());
        assert!(!index_patterns_match(&json!({"index_patterns":"test*,-test3"}), "test3").unwrap());
        assert!(
            index_patterns_match(&json!({"index_patterns":"test3*,-test3,test*"}), "test3")
                .unwrap()
        );
        assert!(index_patterns_match(&json!({"index_patterns":["logs-*"]}), "logs-docs").unwrap());
        assert!(
            !index_patterns_match(&json!({"index_patterns":["metrics-*"]}), "logs-docs").unwrap()
        );
        assert!(index_patterns_match(&json!({"index_patterns":"*"}), "logs-docs").unwrap());
    }

    #[test]
    fn index_patterns_report_unverifiable_shapes() {
        assert!(index_patterns_match(&json!({}), "logs-docs").is_err());
        assert!(index_patterns_match(&json!({"index_patterns": 1}), "logs-docs").is_err());
        assert!(index_patterns_match(&json!({"index_patterns": "-"}), "logs-docs").is_err());
    }

    #[test]
    fn wildcard_matching_supports_zero_or_more_chars() {
        assert!(wildcard_match("logs-*", "logs-docs"));
        assert!(wildcard_match("logs*", "logs"));
        assert!(wildcard_match("*docs", "logs-docs"));
        assert!(!wildcard_match("metrics-*", "logs-docs"));
    }
}
