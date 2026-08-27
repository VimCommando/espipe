use eyre::{Result, eyre};
use rust_embed::RustEmbed;
use serde_json::Value;

#[derive(RustEmbed)]
#[folder = "assets/templates/"]
struct TemplateAssets;

#[derive(Debug)]
pub(super) struct EmbeddedTemplate {
    pub(super) default_name: String,
    pub(super) body: Value,
}

pub(super) fn resolve(selector: &str) -> Result<EmbeddedTemplate> {
    let asset_name = format!("{selector}.yaml");
    let asset = TemplateAssets::get(&asset_name).ok_or_else(|| {
        let available = available().join(", ");
        eyre!("unknown bundled template '{selector}'; available bundled templates: {available}")
    })?;
    let contents = std::str::from_utf8(asset.data.as_ref())
        .map_err(|err| eyre!("bundled template '{selector}' is not UTF-8: {err}"))?;
    let body: Value = yaml_serde::from_str(contents)
        .map_err(|err| eyre!("bundled template '{selector}' is invalid YAML: {err}"))?;
    let default_name = body
        .pointer("/_meta/espipe/default_template_name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            eyre!("bundled template '{selector}' has no _meta.espipe.default_template_name")
        })?
        .to_string();

    Ok(EmbeddedTemplate { default_name, body })
}

pub(super) fn available() -> Vec<String> {
    let mut selectors = TemplateAssets::iter()
        .filter_map(|name| name.strip_suffix(".yaml").map(str::to_string))
        .collect::<Vec<_>>();
    selectors.sort();
    selectors
}

#[cfg(test)]
mod tests {
    use super::{available, resolve};
    use serde_json::Value;

    fn mapping_type(template: &Value, path: &str) -> Option<String> {
        let mut mapping = &template["template"]["mappings"];
        for segment in path.split('.') {
            mapping = mapping.get("properties")?.get(segment)?;
        }
        mapping.get("type")?.as_str().map(str::to_string)
    }

    fn assert_no_multifields(value: &Value) {
        match value {
            Value::Object(object) => {
                assert!(
                    !object.contains_key("fields"),
                    "unexpected multifield: {value}"
                );
                for child in object.values() {
                    assert_no_multifields(child);
                }
            }
            Value::Array(array) => {
                for child in array {
                    assert_no_multifields(child);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn okf_is_compiled_into_the_catalog() {
        let template = resolve("_okf").unwrap();
        assert_eq!(template.default_name, "open-knowledge-format");
        assert_eq!(template.body["_meta"]["okf_version"], "0.2");
        assert!(available().contains(&"_okf".to_string()));
    }

    #[test]
    fn unknown_selector_lists_available_templates() {
        let error = resolve("_missing").unwrap_err().to_string();
        assert!(error.contains("unknown bundled template '_missing'"));
        assert!(error.contains("_okf"));
    }

    #[test]
    fn okf_maps_every_official_v0_2_field() {
        let template = resolve("_okf").unwrap().body;
        let expected = [
            ("content.type", "keyword"),
            ("content.title", "text"),
            ("content.description", "text"),
            ("content.resource", "keyword"),
            ("content.tags", "keyword"),
            ("content.okf_version", "keyword"),
            ("content.status", "keyword"),
            ("content.stale_after", "date"),
            ("content.runtime", "keyword"),
            ("content.computation", "keyword"),
            ("content.body", "text"),
            ("content.markdown", "text"),
            ("content.sources", "nested"),
            ("content.sources.id", "keyword"),
            ("content.sources.resource", "keyword"),
            ("content.sources.title", "text"),
            ("content.sources.author", "keyword"),
            ("content.sources.usage_count", "long"),
            ("content.sources.last_modified", "date"),
            ("content.sources.usage_window.from", "date"),
            ("content.sources.usage_window.to", "date"),
            ("content.usage_window.from", "date"),
            ("content.usage_window.to", "date"),
            ("content.generated.by", "keyword"),
            ("content.generated.at", "date"),
            ("content.verified", "nested"),
            ("content.verified.by", "keyword"),
            ("content.verified.at", "date"),
            ("content.parameters", "nested"),
            ("content.parameters.name", "keyword"),
            ("content.parameters.type", "keyword"),
            ("content.parameters.required", "boolean"),
            ("content.executor.resource", "keyword"),
            ("content.executor.receipt", "keyword"),
            ("content.attester.resource", "keyword"),
            ("origin.scheme", "keyword"),
            ("origin.path", "keyword"),
            ("origin.filename", "keyword"),
        ];

        for (path, expected_type) in expected {
            assert_eq!(
                mapping_type(&template, path).as_deref(),
                Some(expected_type),
                "mapping for {path}"
            );
        }
    }

    #[test]
    fn okf_unknown_strings_are_one_bounded_keyword() {
        let template = resolve("_okf").unwrap().body;
        let mappings = &template["template"]["mappings"];
        assert_eq!(mappings["date_detection"], false);
        assert_eq!(
            mappings["dynamic_templates"][0]["unknown_strings"]["match_mapping_type"],
            "string"
        );
        assert_eq!(
            mappings["dynamic_templates"][0]["unknown_strings"]["mapping"]["type"],
            "keyword"
        );
        assert_eq!(
            mappings["dynamic_templates"][0]["unknown_strings"]["mapping"]["ignore_above"],
            2048
        );
        assert_no_multifields(mappings);
    }

    #[test]
    fn representative_fixture_covers_both_verified_shapes() {
        let documents = include_str!("../../../tests/fixtures/okf_v0_2.ndjson")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(documents.len(), 2);
        assert!(documents[0]["content"]["verified"].is_object());
        assert!(documents[1]["content"]["verified"].is_array());
        for document in documents {
            assert!(document["content"]["sources"][0]["usage_window"].is_object());
            assert!(document["content"]["generated"].is_object());
            assert!(document["content"]["stale_after"].is_string());
            assert!(document["content"]["parameters"].is_array());
            assert!(document["content"]["executor"].is_object());
            assert!(document["content"]["attester"].is_object());
        }
    }
}
