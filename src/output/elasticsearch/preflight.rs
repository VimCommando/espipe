use super::{
    normalized_extension,
    template::{ParsedTemplate, TemplateConfig, install_template, parse_template},
};
use crate::output::OutputPreflightConfig;
use elasticsearch::{
    Elasticsearch,
    http::{
        Method, StatusCode,
        headers::{HeaderMap, HeaderValue},
    },
};
use eyre::{Result, eyre};
use serde_json::Value;
use std::{fs, path::Path};

#[derive(Debug)]
pub(super) struct PreparedPreflight {
    pipeline: Option<NamedJson>,
    template: Option<ParsedTemplate>,
    pub(super) bulk_pipeline: Option<String>,
    template_pipeline: Option<String>,
}

#[derive(Debug)]
struct NamedJson {
    name: String,
    body: Value,
}

impl PreparedPreflight {
    pub(super) fn try_from(config: OutputPreflightConfig) -> Result<Self> {
        let pipeline =
            prepare_pipeline(config.pipeline.as_deref(), config.pipeline_name.as_deref())?;

        let template_config = TemplateConfig::try_new(
            config.template,
            config.template_name,
            config.template_overwrite,
        )?;
        let template = template_config.map(parse_template).transpose()?;

        let template_pipeline = template
            .as_ref()
            .and_then(|template| extract_default_pipeline(&template.body).map(str::to_string));

        validate_template_pipeline(
            template.as_ref(),
            pipeline.as_ref(),
            template_pipeline.as_deref(),
        )?;
        let bulk_pipeline = if template.is_none() {
            pipeline
                .as_ref()
                .map(|pipeline| pipeline.name.clone())
                .or(config.pipeline_name)
        } else {
            None
        };

        Ok(Self {
            pipeline,
            template,
            bulk_pipeline,
            template_pipeline,
        })
    }

    pub(super) async fn run(&self, client: &Elasticsearch, target_index: &str) -> Result<()> {
        if let Some(pipeline) = &self.pipeline {
            put_json(
                client,
                &format!("/_ingest/pipeline/{}", pipeline.name),
                &pipeline.body,
            )
            .await?;
        }

        if let (None, Some(pipeline_name)) = (&self.pipeline, &self.template_pipeline) {
            ensure_pipeline_exists(client, pipeline_name).await?;
        }

        if let Some(template) = &self.template {
            install_template(client, target_index, template).await?;
        }

        Ok(())
    }
}

fn prepare_pipeline(path: Option<&Path>, name: Option<&str>) -> Result<Option<NamedJson>> {
    let pipeline = match (path, name) {
        (Some(path), _) => load_pipeline_config("pipeline", path, name)?,
        (None, None | Some("_none")) => return Ok(None),
        (None, Some(_)) => {
            return Err(eyre!(
                "--pipeline-name requires --pipeline unless the name is _none"
            ));
        }
    };
    if pipeline.name == "_none" {
        return Err(eyre!(
            "_none is reserved for the bulk pipeline target and cannot be installed as an ingest pipeline"
        ));
    }
    Ok(Some(pipeline))
}

fn validate_template_pipeline(
    template: Option<&ParsedTemplate>,
    pipeline: Option<&NamedJson>,
    referenced_name: Option<&str>,
) -> Result<()> {
    let (Some(template), Some(pipeline)) = (template, pipeline) else {
        return Ok(());
    };
    match referenced_name {
        Some(name) if name == pipeline.name => Ok(()),
        Some(name) => Err(eyre!(
            "template references ingest pipeline '{name}', but --pipeline selects '{}'",
            pipeline.name
        )),
        None => Err(eyre!(
            "template '{}' does not reference the provided pipeline '{}'",
            template.name,
            pipeline.name
        )),
    }
}

fn load_pipeline_config(kind: &str, path: &Path, name_override: Option<&str>) -> Result<NamedJson> {
    let contents = fs::read_to_string(path)
        .map_err(|err| eyre!("failed to read {kind} file {}: {err}", path.display()))?;
    let body = parse_pipeline_body(kind, path, &contents)?;
    let name = match name_override {
        Some(name) => name.to_string(),
        None => path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string(),
    };
    if name.is_empty() {
        return Err(eyre!("{kind} name must be non-empty"));
    }
    Ok(NamedJson { name, body })
}

fn parse_pipeline_body(kind: &str, path: &Path, body: &str) -> Result<Value> {
    match normalized_extension(path).as_deref() {
        Some("json") => serde_json::from_str::<Value>(body).map_err(|err| {
            eyre!(
                "failed to parse {kind} file {} as JSON: {err}",
                path.display()
            )
        }),
        Some("yml" | "yaml") => yaml_serde::from_str::<Value>(body).map_err(|err| {
            eyre!(
                "failed to parse {kind} file {} as YAML: {err}",
                path.display()
            )
        }),
        _ => Err(eyre!(
            "{kind} file {} must use the .json, .yml, or .yaml extension",
            path.display()
        )),
    }
}

async fn put_json(client: &Elasticsearch, path: &str, body: &Value) -> Result<()> {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let body = serde_json::to_vec(body)?;
    let response = client
        .send(
            Method::Put,
            path,
            headers,
            Option::<&()>::None,
            Some(body),
            None,
        )
        .await?;
    ensure_success(response.status_code(), response.text().await?, path)
}

async fn ensure_pipeline_exists(client: &Elasticsearch, name: &str) -> Result<()> {
    let response = client
        .send(
            Method::Get,
            &format!("/_ingest/pipeline/{name}"),
            HeaderMap::new(),
            Option::<&()>::None,
            Option::<Vec<u8>>::None,
            None,
        )
        .await?;
    ensure_success(
        response.status_code(),
        response.text().await?,
        &format!("/_ingest/pipeline/{name}"),
    )
    .map_err(|err| {
        eyre!("template references missing or unavailable ingest pipeline '{name}': {err}")
    })
}

fn ensure_success(status: StatusCode, body: String, path: &str) -> Result<()> {
    if status.is_success() {
        Ok(())
    } else {
        Err(eyre!(
            "Elasticsearch request to {path} failed with status {status}: {body}"
        ))
    }
}

fn extract_default_pipeline(template: &Value) -> Option<&str> {
    let settings = template.get("template")?.get("settings")?;
    settings
        .get("index.default_pipeline")
        .and_then(Value::as_str)
        .or_else(|| {
            settings
                .get("index")
                .and_then(|index| index.get("default_pipeline"))
                .and_then(Value::as_str)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn temp_json_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "espipe-pipeline-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            name
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.json"));
        let _ = fs::remove_file(&path);
        path
    }

    #[test]
    fn prepared_preflight_uses_the_pipeline_name_as_the_bulk_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("geoip.json");
        fs::write(&path, r#"{"processors":[]}"#).unwrap();

        for (name_override, expected) in [(None, "geoip"), (Some("normalized"), "normalized")] {
            let preflight = PreparedPreflight::try_from(OutputPreflightConfig {
                pipeline: Some(path.clone()),
                pipeline_name: name_override.map(str::to_string),
                ..OutputPreflightConfig::default()
            })
            .unwrap();
            assert_eq!(preflight.pipeline.as_ref().unwrap().name, expected);
            assert_eq!(preflight.bulk_pipeline.as_deref(), Some(expected));
        }
    }

    #[test]
    fn prepared_preflight_allows_none_without_pipeline_file() {
        let preflight = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline_name: Some("_none".to_string()),
            ..OutputPreflightConfig::default()
        })
        .unwrap();

        assert!(preflight.pipeline.is_none());
        assert_eq!(preflight.bulk_pipeline.as_deref(), Some("_none"));
    }

    #[test]
    fn prepared_preflight_rejects_pipeline_name_without_pipeline_file() {
        let err = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline_name: Some("geoip".to_string()),
            ..OutputPreflightConfig::default()
        })
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--pipeline-name requires --pipeline")
        );
    }

    #[test]
    fn prepared_preflight_rejects_invalid_pipeline_json() {
        let path = temp_json_path("invalid");
        fs::write(&path, "{").unwrap();

        let err = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(path.clone()),
            ..OutputPreflightConfig::default()
        })
        .unwrap_err();

        assert!(err.to_string().contains("failed to parse pipeline file"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn prepared_preflight_accepts_yaml_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("geoip.YML");
        fs::write(
            &path,
            "processors:\n  - set:\n      field: normalized\n      value: true\n",
        )
        .unwrap();

        let preflight = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(path),
            ..OutputPreflightConfig::default()
        })
        .unwrap();

        assert_eq!(preflight.pipeline.as_ref().unwrap().name, "geoip");
        assert_eq!(
            preflight.pipeline.as_ref().unwrap().body["processors"][0]["set"]["field"],
            "normalized"
        );
    }

    #[test]
    fn prepared_preflight_rejects_invalid_pipeline_yaml() {
        let path = std::env::temp_dir().join(format!(
            "espipe-pipeline-test-{}-pipeline.yml",
            std::process::id()
        ));
        fs::write(&path, "processors: [").unwrap();

        let err = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(path.clone()),
            ..OutputPreflightConfig::default()
        })
        .unwrap_err();

        assert!(err.to_string().contains("failed to parse pipeline file"));
        assert!(err.to_string().contains("as YAML"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn prepared_preflight_rejects_unsupported_pipeline_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pipeline.jsonc");
        fs::write(&path, r#"{"processors":[]}"#).unwrap();

        let err = PreparedPreflight::try_from(OutputPreflightConfig {
            pipeline: Some(path),
            ..OutputPreflightConfig::default()
        })
        .unwrap_err();

        assert!(err.to_string().contains(".json, .yml, or .yaml"));
    }

    #[test]
    fn extract_default_pipeline_supports_nested_and_flattened_settings() {
        let nested = json!({
            "template": {
                "settings": {
                    "index": {
                        "default_pipeline": "geoip"
                    }
                }
            }
        });
        assert_eq!(extract_default_pipeline(&nested), Some("geoip"));

        let flattened = json!({
            "template": {
                "settings": {
                    "index.default_pipeline": "normalized"
                }
            }
        });
        assert_eq!(extract_default_pipeline(&flattened), Some("normalized"));
    }

    #[test]
    fn prepared_preflight_validates_the_pipeline_referenced_by_the_template() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = dir.path().join("geoip.json");
        let template = dir.path().join("template.json");
        fs::write(&pipeline, r#"{"processors":[]}"#).unwrap();
        let config = || OutputPreflightConfig {
            pipeline: Some(pipeline.clone()),
            template: Some(template.clone()),
            ..OutputPreflightConfig::default()
        };
        let write_template = |name| {
            fs::write(
                &template,
                json!({
                    "template": {"settings": {"index.default_pipeline": name}}
                })
                .to_string(),
            )
            .unwrap();
        };

        write_template("other");
        let err = PreparedPreflight::try_from(config())
            .unwrap_err()
            .to_string();
        assert!(err.contains("other") && err.contains("geoip"), "{err}");

        write_template("geoip");
        let preflight = PreparedPreflight::try_from(config()).unwrap();
        assert_eq!(preflight.pipeline.as_ref().unwrap().name, "geoip");
        assert_eq!(preflight.template_pipeline.as_deref(), Some("geoip"));
        assert!(preflight.bulk_pipeline.is_none());
    }
}
