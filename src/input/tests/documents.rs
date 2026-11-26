use super::collect_values;
use super::read_err;
use super::uri;
use super::workspace_tempdir;
use crate::input::Input;
use crate::input::open_input_values;
use crate::input::stream::JSON_LINE_OPENING_ERROR;
use crate::input::validate_content_field;
use serde_json::{Value, json};
use std::fs;

#[test]
fn direct_markdown_file_imports_default_content_body() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    fs::write(&path, "# Title\nBody\n").unwrap();

    let values = collect_values(Input::try_from(uri(&path)).unwrap());

    let document = single_file_document(&values, "note.md");
    assert_eq!(document["content"]["body"], "# Title\nBody\n");
}

#[test]
fn content_field_validation_rejects_empty_and_dotted_names() {
    for valid in ["body", "markdown"] {
        assert!(validate_content_field(valid).is_ok());
    }
    for (invalid, reason) in [("", "empty"), ("page.body", "must not contain")] {
        let err = validate_content_field(invalid).unwrap_err();
        assert!(err.to_string().contains(reason));
    }
}

#[test]
fn custom_content_field_is_used_without_body() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    fs::write(&path, "hello").unwrap();

    let values = collect_values(open_input_values(vec![uri(&path)], "markdown").unwrap());
    let document = single_file_document(&values, "note.txt");

    assert_eq!(document["content"], json!({"markdown": "hello"}));
}

fn single_file_document<'a>(values: &'a [Value], filename: &str) -> &'a Value {
    let [document] = values else {
        panic!("expected one document for {filename}, got {values:?}");
    };
    let origin = &document["origin"];
    assert_eq!(
        (&origin["scheme"], &origin["filename"]),
        (&json!("file"), &json!(filename))
    );
    document
}

#[test]
fn single_direct_file_document_includes_origin_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    fs::write(&path, "hello").unwrap();

    let values = collect_values(open_input_values(vec![uri(&path)], "body").unwrap());

    assert_eq!(values[0]["origin"]["scheme"], "file");
    assert_eq!(values[0]["origin"]["filename"], "note.txt");
}

#[test]
fn markdown_frontmatter_is_extracted_and_conflicts_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    fs::write(&path, "---\ntitle: Hello\ntags:\n  - docs\n---\n# Body\n").unwrap();

    let values = collect_values(open_input_values(vec![uri(&path)], "body").unwrap());

    assert_eq!(values[0]["content"]["title"], "Hello");
    assert_eq!(values[0]["content"]["tags"], serde_json::json!(["docs"]));
    assert_eq!(values[0]["content"]["body"], "# Body\n");

    fs::write(&path, "---\nbody: duplicate\n---\n# Body\n").unwrap();
    let err = read_err(open_input_values(vec![uri(&path)], "body"));
    assert!(err.contains("conflicts with content field 'body'"));
}

#[test]
fn markdown_frontmatter_closing_delimiter_can_end_at_eof() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    fs::write(&path, "---\ntitle: Hello\n---").unwrap();

    let values = collect_values(open_input_values(vec![uri(&path)], "body").unwrap());

    assert_eq!(values[0]["content"]["title"], "Hello");
    assert_eq!(values[0]["content"]["body"], "");
}

#[test]
fn markdown_duplicate_frontmatter_keys_warn_and_use_last_value() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    fs::write(
        &path,
        "---\nnavigation_title: First\nnavigation_title: Second\n---\n# Body\n",
    )
    .unwrap();

    let values = collect_values(open_input_values(vec![uri(&path)], "body").unwrap());

    assert_eq!(values[0]["content"]["navigation_title"], "Second");
    assert_eq!(values[0]["content"]["body"], "# Body\n");
}

#[test]
fn markdown_non_mapping_frontmatter_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    fs::write(&path, "---\n- bad\n---\n# Body\n").unwrap();

    let err = read_err(open_input_values(vec![uri(&path)], "body"));

    assert!(err.contains("invalid frontmatter"));
}

#[test]
fn yaml_mapping_imports_under_content_and_non_mapping_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.yml");
    fs::write(&path, "title: Hello\ncount: 2\n").unwrap();

    let values = collect_values(open_input_values(vec![uri(&path)], "body").unwrap());

    let document = single_file_document(&values, "doc.yml");
    assert_eq!(document["content"], json!({"count": 2, "title": "Hello"}));

    fs::write(&path, "- bad\n").unwrap();
    let err = read_err(open_input_values(vec![uri(&path)], "body"));
    assert!(err.contains("invalid YAML document shape"));
}

#[test]
fn yaml_mapping_rejects_content_field_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.yml");
    fs::write(&path, "markdown: duplicate\n").unwrap();

    let err = read_err(open_input_values(vec![uri(&path)], "markdown"));

    assert!(err.contains("conflicts with content field 'markdown'"));
}

#[test]
fn batch_file_import_skips_unreadable_files_after_reading_earlier_files() {
    let dir = workspace_tempdir();
    let first = dir.path().join("a.txt");
    let second = dir.path().join("b.txt");
    fs::write(&first, "alpha").unwrap();
    fs::write(&second, [0xff]).unwrap();

    let mut input = open_input_values(vec![uri(&first), uri(&second)], "body").unwrap();
    let mut line = String::new();

    let value = input.read_line(&mut line).unwrap();
    let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
    assert_eq!(actual["content"]["body"], "alpha");

    line.clear();
    assert!(input.read_next(&mut line).unwrap().is_none());
}

#[test]
fn json_file_import_rejects_non_object() {
    let dir = workspace_tempdir();
    let path = dir.path().join("doc.json");
    fs::write(&path, "{\"a\":1}").unwrap();

    let values = collect_values(open_input_values(vec![uri(&path), uri(&path)], "body").unwrap());
    let document = single_file_document(&values, "doc.json");
    assert_eq!(document["a"], 1);

    fs::write(&path, "[1,2]").unwrap();
    let err = read_err(open_input_values(vec![uri(&path)], "body"));
    assert_eq!(err, JSON_LINE_OPENING_ERROR);
}

#[test]
fn jsonl_streams_object_lines_and_rejects_non_objects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.jsonl");
    fs::write(&path, "{\"a\":1}\n\n{\"b\":2}\n").unwrap();

    let values = collect_values(open_input_values(vec![uri(&path)], "body").unwrap());
    let [first, second] = values.as_slice() else {
        panic!("expected two JSONL records, got {values:?}");
    };
    assert_eq!((&first["a"], &second["b"]), (&json!(1), &json!(2)));
    for value in &values {
        assert_eq!(value["origin"]["filename"], "doc.jsonl");
    }

    fs::write(&path, "[1,2]\n").unwrap();
    let err = read_err(open_input_values(vec![uri(&path)], "body"));
    assert!(err.contains("JSON line must be an object"));
}

#[test]
fn invalid_utf8_file_document_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.txt");
    fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();

    let err = read_err(open_input_values(vec![uri(&path)], "body"));

    assert!(err.contains("not valid UTF-8"));
}
