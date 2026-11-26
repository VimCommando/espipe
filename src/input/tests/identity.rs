use super::collect_documents;
use super::fixture_path;
use super::uri;
use crate::input::Input;
use crate::input::documents::open_file_documents;
use crate::input::metadata::DocumentDiscriminator;
use crate::input::metadata::file_document_id_for_relative_path;
use crate::input::metadata::normalize_local_path;
use crate::input::metadata::origin_from_local_path;
use crate::input::metadata::origin_from_uri;
use crate::input::metadata::relative_path_from_working_dir;
use crate::input::open_input_values_with_generate_id;
use fluent_uri::UriRef;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[test]
fn generated_file_id_is_stable_when_content_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    fs::write(&path, "first").unwrap();

    let first = collect_documents(open_file_documents(vec![uri(&path)], "body", true).unwrap());
    let first_id = first[0].generated_id.clone().unwrap();
    assert_eq!(first_id.len(), 22);

    fs::write(&path, "second").unwrap();
    let second = collect_documents(open_file_documents(vec![uri(&path)], "body", true).unwrap());
    assert_eq!(second[0].generated_id.as_deref(), Some(first_id.as_str()));
}

#[test]
fn direct_structured_local_files_include_origin_and_generated_ids() {
    let dir = tempfile::tempdir().unwrap();
    let json = dir.path().join("document.json");
    let ndjson = dir.path().join("documents.ndjson");
    let csv = dir.path().join("documents.csv");
    fs::write(&json, r#"{"value":1}"#).unwrap();
    fs::write(&ndjson, "{\"value\":1}\n{\"value\":2}\n").unwrap();
    fs::write(&csv, "value\n1\n").unwrap();

    let disabled = collect_documents(
        open_input_values_with_generate_id(vec![uri(&json)], "body", Some(false)).unwrap(),
    );
    assert!(
        disabled
            .iter()
            .all(|document| document.generated_id.is_none())
    );

    for path in [json, ndjson, csv, fixture_path("single.toon")] {
        let default_documents = collect_documents(Input::try_from(uri(&path)).unwrap());
        assert!(
            !default_documents.is_empty(),
            "{} produced no documents",
            path.display()
        );
        for document in &default_documents {
            let value: serde_json::Value = serde_json::from_str(document.get()).unwrap();
            assert_eq!(value["origin"]["scheme"], "file");
            assert_eq!(
                value["origin"]["filename"],
                path.file_name().unwrap().to_str().unwrap()
            );
            assert!(document.generated_id.is_none());
        }

        let documents = collect_documents(
            open_input_values_with_generate_id(vec![uri(&path)], "body", Some(true)).unwrap(),
        );
        for document in documents {
            assert!(document.generated_id.is_some());
        }
    }
}

#[test]
fn single_source_external_file_can_opt_into_generated_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("external.md");
    fs::write(&path, "external").unwrap();

    let documents = collect_documents(
        open_input_values_with_generate_id(vec![uri(&path)], "body", Some(true)).unwrap(),
    );
    assert_eq!(documents.len(), 1);
    assert!(documents[0].generated_id.is_some());
}

#[test]
fn generated_file_id_is_stable_across_checkout_roots() {
    let first_path = relative_path_from_working_dir(
        Path::new("/checkouts/first/bundle/docs/getting-started.md"),
        Path::new("/checkouts/first/bundle"),
    );
    let second_path = relative_path_from_working_dir(
        Path::new("/worktrees/second/bundle/docs/getting-started.md"),
        Path::new("/worktrees/second/bundle"),
    );
    assert_eq!(first_path, second_path);

    let first =
        file_document_id_for_relative_path("bundle", &first_path, DocumentDiscriminator::Record(0))
            .unwrap();
    let second = file_document_id_for_relative_path(
        "bundle",
        &second_path,
        DocumentDiscriminator::Record(0),
    )
    .unwrap();

    assert_eq!(first, second);
}

#[cfg(unix)]
#[test]
fn normalize_local_path_preserves_root_when_parent_escapes_it() {
    assert_eq!(
        normalize_local_path(Path::new("/../etc/passwd")).unwrap(),
        PathBuf::from("/etc/passwd")
    );
}

#[test]
fn generated_file_id_uses_compact_128_bit_base64url_encoding() {
    let id = file_document_id_for_relative_path(
        "bundle",
        Path::new("docs/getting-started.md"),
        DocumentDiscriminator::Record(0),
    )
    .unwrap();

    assert_eq!(id.len(), 22);
    assert!(
        id.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    );
}

#[test]
fn generated_file_id_is_not_emitted_for_explicit_id_or_when_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let explicit_path = dir.path().join("explicit.json");
    fs::write(&explicit_path, r#"{"_id":"provided","value":1}"#).unwrap();
    let explicit =
        collect_documents(open_file_documents(vec![uri(&explicit_path)], "body", true).unwrap());
    assert!(explicit[0].generated_id.is_none());

    let path = dir.path().join("note.md");
    fs::write(&path, "hello").unwrap();
    let disabled = collect_documents(open_file_documents(vec![uri(&path)], "body", false).unwrap());
    assert!(disabled[0].generated_id.is_none());
}

#[test]
fn generated_file_ids_include_document_index_for_multi_document_files() {
    let path = fixture_path("multi.toon");
    let documents = collect_documents(open_file_documents(vec![uri(&path)], "body", true).unwrap());

    assert_eq!(documents.len(), 2);
    assert_ne!(documents[0].generated_id, documents[1].generated_id);
    assert_eq!(documents[0].generated_id.as_ref().unwrap().len(), 22);
    assert_eq!(documents[1].generated_id.as_ref().unwrap().len(), 22);
}

#[test]
fn origin_metadata_uses_root_for_relative_paths_and_omits_null_values() {
    let value = origin_from_local_path(Path::new("document.pdf")).into_value();

    assert_eq!(value["scheme"], "file");
    assert_eq!(value["path"], "./");
    assert_eq!(value["filename"], "document.pdf");
    assert!(value.get("authority").is_none());
    assert!(value.get("query").is_none());
    assert!(value.get("fragment").is_none());
}

#[test]
fn origin_metadata_handles_uri_root_and_trailing_slash() {
    let root =
        origin_from_uri(&UriRef::parse("https://example.com/".to_string()).unwrap()).into_value();
    assert_eq!(root["path"], "/");
    assert_eq!(root["filename"], "");

    let directory =
        origin_from_uri(&UriRef::parse("https://example.com/docs/".to_string()).unwrap())
            .into_value();
    assert_eq!(directory["path"], "/docs/");
    assert_eq!(directory["filename"], "");
}
