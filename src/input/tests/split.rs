use super::collect_documents;
use super::fixture_path;
use super::input_err;
use super::uri;
use super::workspace_tempdir;
use crate::input::split::open_split_inputs;
use crate::json_split::SplitPath;
use std::fs;

#[test]
fn split_documents_include_origin_and_typed_generated_ids() {
    let map_path = fixture_path("split_root_map.json");
    let map_documents = collect_documents(
        open_split_inputs(
            vec![uri(&map_path)],
            SplitPath::parse("/").unwrap(),
            Some(true),
        )
        .unwrap(),
    );
    assert_eq!(map_documents.len(), 2);
    assert!(map_documents.iter().all(|document| {
        let value: serde_json::Value = serde_json::from_str(document.get()).unwrap();
        value["origin"]["scheme"] == "file" && document.generated_id.is_some()
    }));
    assert_ne!(map_documents[0].generated_id, map_documents[1].generated_id);

    let array_path = fixture_path("split_nested_array.json");
    let array_documents = collect_documents(
        open_split_inputs(
            vec![uri(&array_path)],
            SplitPath::parse("/hits").unwrap(),
            Some(true),
        )
        .unwrap(),
    );
    assert_eq!(array_documents.len(), 2);
    assert_ne!(
        array_documents[0].generated_id,
        array_documents[1].generated_id
    );
}

#[test]
fn split_applies_per_file_for_multi_source_inputs() {
    let dir = workspace_tempdir();
    let first = dir.path().join("first.json");
    let second = dir.path().join("second.json");
    fs::copy(fixture_path("split_root_map.json"), &first).unwrap();
    fs::copy(fixture_path("split_root_map.json"), &second).unwrap();

    let documents = collect_documents(
        open_split_inputs(
            vec![uri(&first), uri(&second)],
            SplitPath::parse("/").unwrap(),
            None,
        )
        .unwrap(),
    );
    assert_eq!(documents.len(), 4);
    assert!(documents.iter().all(|document| {
        let value: serde_json::Value = serde_json::from_str(document.get()).unwrap();
        value["origin"]["scheme"] == "file" && document.generated_id.is_some()
    }));
    assert_ne!(documents[0].generated_id, documents[2].generated_id);
}

#[test]
fn split_rejects_ndjson_inputs_before_opening() {
    let dir = workspace_tempdir();
    let input = dir.path().join("records.ndjson");
    fs::write(&input, "{\"message\":\"hello\"}\n").unwrap();

    let err = input_err(open_split_inputs(
        vec![uri(&input)],
        SplitPath::parse("/").unwrap(),
        None,
    ));

    assert!(err.contains("--split requires a JSON input source"));
}

#[test]
fn batch_split_continues_after_a_parser_error() {
    let dir = workspace_tempdir();
    let first = dir.path().join("a.json");
    let bad = dir.path().join("b.json");
    let last = dir.path().join("c.json");
    fs::write(&first, r#"[{"value":1}]"#).unwrap();
    fs::write(&bad, "[").unwrap();
    fs::write(&last, r#"[{"value":3}]"#).unwrap();
    let input = open_split_inputs(
        vec![uri(&first), uri(&bad), uri(&last)],
        SplitPath::parse("/").unwrap(),
        None,
    )
    .unwrap();

    let values: Vec<_> = collect_documents(input)
        .into_iter()
        .map(|doc| serde_json::from_str::<serde_json::Value>(doc.get()).unwrap()["value"].clone())
        .collect();
    assert_eq!(values, [1, 3]);
}
