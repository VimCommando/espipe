use super::collect_values;
use super::fixture_path;
use super::input_err;
use super::read_err;
use super::temp_path;
use super::uri;
use super::workspace_tempdir;
use super::write_gzip;
use crate::input::Input;
use crate::input::open_input_values;
use crate::input::stream::InputKind;
use crate::input::stream::JSON_LINE_OPENING_ERROR;
use crate::input::stream::input_kind_from_path;
use crate::input::stream::local_input_kind;
use fluent_uri::UriRef;
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

#[test]
fn input_kind_detects_supported_compressed_suffixes() {
    for (suffix, expected) in [
        ("csv.gz", Some(InputKind::Csv)),
        ("ndjson.gz", Some(InputKind::Ndjson)),
        ("json.gz", None),
        ("csv", Some(InputKind::Csv)),
        ("ndjson", Some(InputKind::Ndjson)),
        ("json", Some(InputKind::Json)),
        ("toon", Some(InputKind::Toon)),
        ("toon.gz", None),
    ] {
        assert_eq!(
            input_kind_from_path(&format!("/tmp/events.{suffix}")),
            expected
        );
    }
}

#[test]
fn csv_and_ndjson_streams_include_file_origin_with_or_without_gzip() {
    let dir = tempfile::tempdir().unwrap();
    for (filename, contents, expected) in [
        (
            "a.csv",
            "name,count\nalpha,2\n",
            json!({"name":"alpha", "count":"2"}),
        ),
        (
            "a.csv.gz",
            "name,count\nalpha,2\n",
            json!({"name":"alpha", "count":"2"}),
        ),
        ("a.ndjson", "{\"a\":1}\n", json!({"a":1})),
        ("a.ndjson.gz", "{\"a\":1}\n", json!({"a":1})),
    ] {
        let path = dir.path().join(filename);
        if filename.ends_with(".gz") {
            write_gzip(&path, contents);
        } else {
            fs::write(&path, contents).unwrap();
        }
        let mut input = Input::try_from(uri(&path)).unwrap();
        let document = input.read_line(&mut String::new()).unwrap();
        let mut actual: Value = serde_json::from_str(document.get()).unwrap();
        let origin = actual.as_object_mut().unwrap().remove("origin").unwrap();

        assert_eq!(actual, expected, "{filename}");
        assert_eq!(
            (&origin["scheme"], &origin["filename"]),
            (&json!("file"), &json!(filename))
        );
    }
}

#[test]
fn gzip_json_input_is_rejected_as_unsupported() {
    let path = temp_path("json.gz");
    write_gzip(&path, "{\"a\":1}\n");

    let err = input_err(Input::try_from(
        UriRef::parse(path.to_string_lossy().into_owned()).unwrap(),
    ));

    assert!(err.contains("Unsupported compressed input format"));
    fs::remove_file(path).unwrap();
}

#[test]
fn empty_local_streaming_input_still_reports_one_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.ndjson");
    fs::write(&path, "").unwrap();

    let input = Input::try_from(uri(&path)).unwrap();

    assert_eq!(input.file_count(), Some(1));
}

#[test]
fn toon_file_streams_object_documents_in_order() {
    let values = collect_values(Input::try_from(uri(&fixture_path("multi.toon"))).unwrap());

    let documents: Vec<_> = values
        .iter()
        .map(|value| (&value["id"], &value["name"], &value["origin"]["filename"]))
        .collect();
    assert_eq!(
        documents,
        [
            (&json!(1), &json!("Alpha"), &json!("multi.toon")),
            (&json!(2), &json!("Bravo"), &json!("multi.toon")),
        ]
    );
    assert_eq!(values[1]["tags"], json!(["search", "bulk"]));
}

#[test]
fn toon_root_tabular_array_emits_one_document_per_row() {
    let values = collect_values(Input::try_from(uri(&fixture_path("measurements.toon"))).unwrap());
    let [baseline, candidate, throughput] = values.as_slice() else {
        panic!("expected three Toon rows, got {values:?}");
    };

    assert_eq!(
        (
            &baseline["@timestamp"],
            &baseline["evaluation"],
            &baseline["metric"],
            baseline["value"].as_f64()
        ),
        (
            &json!("2026-05-06T18:42:00Z"),
            &json!("force-merge-20260506T184200Z"),
            &json!("search_latency_p99"),
            Some(100.0)
        ),
    );
    assert_eq!(
        (&candidate["variation"], candidate["value"].as_f64()),
        (&json!("candidate"), Some(150.0))
    );
    assert_eq!(
        (
            &throughput["metric"],
            &throughput["unit"],
            &throughput["artifact"]
        ),
        (
            &json!("throughput"),
            &json!("docs/s"),
            &json!("comparison.toon")
        ),
    );
}

#[test]
fn toon_root_tabular_array_rejects_non_object_rows() {
    let path = temp_path("toon");
    fs::write(&path, "items[2]: a,b\n").unwrap();

    let err = read_err(Input::try_from(uri(&path)));

    assert!(err.contains("Toon array row must be an object"));
    fs::remove_file(path).unwrap();
}

#[test]
fn single_toon_file_imports_one_object_document() {
    let values = collect_values(Input::try_from(uri(&fixture_path("single.toon"))).unwrap());

    let [document] = values.as_slice() else {
        panic!("expected one Toon document, got {values:?}");
    };
    assert_eq!(
        (
            &document["active"],
            &document["id"],
            &document["name"],
            &document["origin"]["filename"]
        ),
        (
            &json!(true),
            &json!(1),
            &json!("Alpha"),
            &json!("single.toon")
        ),
    );
}

#[test]
fn toon_file_rejects_malformed_and_non_object_documents() {
    let malformed = read_err(Input::try_from(uri(&fixture_path("malformed.toon"))));
    assert!(malformed.contains("invalid Toon document"));

    let non_object = read_err(Input::try_from(uri(&fixture_path("non_object.toon"))));
    assert!(non_object.contains("Toon document must be an object"));

    let scalar = temp_path("toon");
    fs::write(&scalar, "true\n").unwrap();
    let scalar_err = read_err(Input::try_from(uri(&scalar)));
    assert!(scalar_err.contains("Toon document must be an object"));
    fs::remove_file(scalar).unwrap();
}

#[test]
fn toon_stream_stops_on_parse_failure_after_valid_document() {
    let path = temp_path("toon");
    fs::write(&path, "id: 1\n---\nitems[2]: a\n").unwrap();
    let mut input = Input::try_from(uri(&path)).unwrap();
    let mut line = String::new();

    let first = input.read_line(&mut line).unwrap();
    let actual: serde_json::Value = serde_json::from_str(first.get()).unwrap();
    assert_eq!(actual["id"], 1);
    assert_eq!(actual["origin"]["scheme"], "file");
    assert_eq!(
        actual["origin"]["filename"],
        path.file_name().unwrap().to_str().unwrap()
    );

    line.clear();
    let err = input.read_line(&mut line).unwrap_err().to_string();
    assert!(err.contains("invalid Toon document"));

    fs::remove_file(path).unwrap();
}

#[test]
fn toon_file_in_multi_input_includes_origin_metadata() {
    let dir = workspace_tempdir();
    let text = dir.path().join("a.txt");
    let toon = dir.path().join("b.toon");
    fs::write(&text, "alpha").unwrap();
    fs::write(&toon, "id: 2\nname: Bravo\n").unwrap();

    let values = collect_values(open_input_values(vec![uri(&text), uri(&toon)], "body").unwrap());

    assert_eq!(values.len(), 2);
    let text_value = values
        .iter()
        .find(|value| value["origin"]["filename"] == "a.txt")
        .unwrap();
    let toon_value = values
        .iter()
        .find(|value| value["origin"]["filename"] == "b.toon")
        .unwrap();
    assert_eq!(text_value["content"]["body"], "alpha");
    assert_eq!(toon_value["id"], 2);
}

#[test]
fn read_line_rejects_json_arrays() {
    let path = temp_path("ndjson");
    fs::write(&path, "[1,2]\n").unwrap();
    let mut input =
        Input::try_from(UriRef::parse(path.to_string_lossy().into_owned()).unwrap()).unwrap();

    let mut line = String::new();
    let err = input.read_line(&mut line).unwrap_err();
    assert_eq!(err.to_string(), JSON_LINE_OPENING_ERROR);

    fs::remove_file(path).unwrap();
}

#[test]
fn existing_stdin_marker_is_preserved() {
    let input = Input::try_from(UriRef::parse("-".to_string()).unwrap()).unwrap();

    assert!(matches!(input, Input::Stdin { .. }));
}

#[test]
fn existing_local_json_stream_behavior_is_preserved_for_single_input() {
    let path = temp_path("json");
    fs::write(&path, "{\"a\":1}\n{\"b\":2}\n").unwrap();
    let mut input =
        Input::try_from(UriRef::parse(path.to_string_lossy().into_owned()).unwrap()).unwrap();

    let mut line = String::new();
    let first = input.read_line(&mut line).unwrap();
    let first_value: serde_json::Value = serde_json::from_str(first.get()).unwrap();
    assert_eq!(first_value["a"], 1);
    assert_eq!(first_value["origin"]["scheme"], "file");
    line.clear();
    let second = input.read_line(&mut line).unwrap();
    let second_value: serde_json::Value = serde_json::from_str(second.get()).unwrap();
    assert_eq!(second_value["b"], 2);
    assert_eq!(second_value["origin"]["scheme"], "file");

    fs::remove_file(path).unwrap();
}

#[test]
fn single_line_json_file_is_processed_as_one_document() {
    let path = temp_path("json");
    fs::write(&path, "{\"a\":1}").unwrap();
    let mut input =
        Input::try_from(UriRef::parse(path.to_string_lossy().into_owned()).unwrap()).unwrap();

    let mut line = String::new();
    let value = input.read_line(&mut line).unwrap();
    let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
    assert_eq!(actual["a"], 1);
    assert_eq!(actual["origin"]["scheme"], "file");

    fs::remove_file(path).unwrap();
}

#[test]
fn pretty_json_file_is_processed_as_one_document_when_first_line_is_open_brace() {
    let path = temp_path("json");
    fs::write(&path, "{\n  \"a\": 1,\n  \"b\": {\n    \"c\": 2\n  }\n}\n").unwrap();
    let mut input =
        Input::try_from(UriRef::parse(path.to_string_lossy().into_owned()).unwrap()).unwrap();

    let mut line = String::new();
    let value = input.read_line(&mut line).unwrap();
    let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
    assert_eq!(actual["a"], 1);
    assert_eq!(actual["b"]["c"], 2);
    assert_eq!(actual["origin"]["scheme"], "file");

    line.clear();
    assert_eq!(
        input.read_line(&mut line).unwrap_err().to_string(),
        "No JSON record"
    );

    fs::remove_file(path).unwrap();
}

#[test]
fn unsupported_input_scheme_is_rejected() {
    let uri = UriRef::parse("ftp://example.com/data.ndjson".to_string()).unwrap();
    match Input::try_from(uri) {
        Ok(_) => panic!("ftp input should be rejected"),
        Err(err) => assert!(err.to_string().contains("Unsupported input scheme: ftp")),
    }
}

#[test]
fn json_extension_is_accepted_for_local_input_detection() {
    let path = PathBuf::from("/tmp/example.json");
    let kind = local_input_kind(&path).unwrap();
    assert_eq!(kind, InputKind::Json);
}
