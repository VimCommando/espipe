use super::collect_documents;
use super::collect_values;
use super::input_err;
use super::uri;
use super::workspace_tempdir;
use super::write_gzip;
use crate::input::DiscoveryOptions;
use crate::input::HiddenMode;
use crate::input::Input;
use crate::input::SymlinkMode;
use crate::input::metadata::DocumentDiscriminator;
use crate::input::metadata::bundle_identifier;
use crate::input::metadata::file_document_id;
use crate::input::metadata::normalize_local_path;
use crate::input::open_input_values;
use crate::input::open_input_values_with_generate_id_and_options;
use fluent_uri::UriRef;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::Path;

#[test]
fn multi_source_discovery_counts_paths_without_constructing_input() {
    let directory = workspace_tempdir();
    let first = directory.path().join("first.ndjson");
    let second = directory.path().join("second.ndjson");
    fs::write(&first, "{\"value\":1}\n").unwrap();
    fs::write(&second, "{\"value\":2}\n").unwrap();
    let first = UriRef::parse(first.display().to_string()).unwrap();
    let second = UriRef::parse(second.display().to_string()).unwrap();

    assert!(
        Input::discover_is_multi_source_local(
            &[first.clone(), second],
            DiscoveryOptions::default()
        )
        .unwrap()
    );
    assert!(!Input::discover_is_multi_source_local(&[first], DiscoveryOptions::default()).unwrap());
}

#[test]
fn gzip_json_glob_input_is_rejected_as_unsupported() {
    let dir = workspace_tempdir();
    let path = dir.path().join("doc.json.gz");
    write_gzip(&path, "{\"a\":1}\n");
    let pattern = dir.path().join("*.gz").to_string_lossy().into_owned();

    let err = input_err(open_input_values(
        vec![UriRef::parse(pattern).unwrap()],
        "body",
    ));

    assert!(err.contains("Unsupported compressed input format"));
}

#[test]
fn gzip_json_multi_input_is_rejected_as_unsupported() {
    let dir = workspace_tempdir();
    let good = dir.path().join("doc.txt");
    let bad = dir.path().join("doc.ndjson.gz");
    fs::write(&good, "hello").unwrap();
    write_gzip(&bad, "{\"a\":1}\n");

    let err = input_err(open_input_values(vec![uri(&good), uri(&bad)], "body"));

    assert!(err.contains("Unsupported compressed input format"));
}

#[test]
fn multi_source_inputs_reject_external_paths() {
    let dir = tempfile::Builder::new()
        .prefix("espipe-external-")
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap())
        .unwrap();
    let first = dir.path().join("first.txt");
    let second = dir.path().join("second.txt");
    fs::write(&first, "first").unwrap();
    fs::write(&second, "second").unwrap();

    let err = input_err(open_input_values(vec![uri(&first), uri(&second)], "body"));
    assert!(err.contains("outside the working directory"));
}

#[cfg(unix)]
#[test]
fn multi_source_inputs_skip_symlink_escapes_by_default() {
    let dir = workspace_tempdir();
    let external = tempfile::Builder::new()
        .prefix("espipe-external-")
        .tempdir_in(std::env::temp_dir())
        .unwrap();
    let external_file = external.path().join("external.txt");
    fs::write(&external_file, "external").unwrap();
    let link = dir.path().join("link.txt");
    symlink(&external_file, &link).unwrap();
    let local = dir.path().join("local.txt");
    fs::write(&local, "local").unwrap();

    let documents =
        collect_documents(open_input_values(vec![uri(&link), uri(&local)], "body").unwrap());
    assert_eq!(documents.len(), 1);
    let value: serde_json::Value = serde_json::from_str(documents[0].get()).unwrap();
    assert_eq!(value["origin"]["filename"], "local.txt");
    assert!(documents[0].generated_id.is_none());
}

#[cfg(unix)]
#[test]
fn multi_source_inputs_can_follow_external_symlinks_with_lexical_identity() {
    let dir = workspace_tempdir();
    let external = tempfile::Builder::new()
        .prefix("espipe-external-")
        .tempdir_in(std::env::temp_dir())
        .unwrap();
    let external_file = external.path().join("external.txt");
    fs::write(&external_file, "external").unwrap();
    let link = dir.path().join("link.txt");
    symlink(&external_file, &link).unwrap();
    let local = dir.path().join("local.txt");
    fs::write(&local, "local").unwrap();

    let documents = collect_documents(
        open_input_values_with_generate_id_and_options(
            vec![uri(&link), uri(&local)],
            "body",
            None,
            DiscoveryOptions {
                symlinks: SymlinkMode::Follow,
                hidden: HiddenMode::Skip,
            },
        )
        .unwrap(),
    );
    let link_document = documents
        .iter()
        .find(|document| {
            let value: serde_json::Value = serde_json::from_str(document.get()).unwrap();
            value["origin"]["filename"] == "link.txt"
        })
        .unwrap();
    let expected_id = file_document_id(
        &bundle_identifier().unwrap(),
        &normalize_local_path(&link).unwrap(),
        DocumentDiscriminator::Record(0),
    )
    .unwrap();
    assert_eq!(
        link_document.generated_id.as_deref(),
        Some(expected_id.as_str())
    );
    let value: serde_json::Value = serde_json::from_str(link_document.get()).unwrap();
    assert_eq!(value["origin"]["filename"], "link.txt");
}

#[cfg(unix)]
#[test]
fn multi_source_inputs_apply_symlink_policy_and_working_directory_boundary() {
    let dir = workspace_tempdir();
    let external = tempfile::Builder::new()
        .prefix("espipe-external-")
        .tempdir_in(std::env::temp_dir())
        .unwrap();
    let external_file = external.path().join("external.txt");
    fs::write(&external_file, "external").unwrap();
    let local = dir.path().join("local.txt");
    fs::write(&local, "local").unwrap();

    for (parent, mode, expected_error) in [
        (dir.path(), SymlinkMode::Fail, "Symlink path encountered"),
        (
            external.path(),
            SymlinkMode::Follow,
            "outside the working directory",
        ),
    ] {
        let link = parent.join("link.txt");
        symlink(&external_file, &link).unwrap();
        let err = input_err(open_input_values_with_generate_id_and_options(
            vec![uri(&link), uri(&local)],
            "body",
            None,
            DiscoveryOptions {
                symlinks: mode,
                hidden: HiddenMode::Skip,
            },
        ));
        assert!(err.contains(expected_error), "{err}");
    }
}

#[test]
fn multi_source_inputs_apply_hidden_path_policy_before_cardinality() {
    let dir = workspace_tempdir();
    let hidden_dir = dir.path().join(".private");
    fs::create_dir(&hidden_dir).unwrap();
    let hidden_file = dir.path().join(".hidden.txt");
    let nested_hidden_file = hidden_dir.join("nested.txt");
    let visible_file = dir.path().join("visible.txt");
    fs::write(&hidden_file, "hidden").unwrap();
    fs::write(&nested_hidden_file, "nested hidden").unwrap();
    fs::write(&visible_file, "visible").unwrap();

    let default_documents = collect_documents(
        open_input_values(
            vec![
                uri(&hidden_file),
                uri(&nested_hidden_file),
                uri(&visible_file),
            ],
            "body",
        )
        .unwrap(),
    );
    assert_eq!(default_documents.len(), 1);
    assert!(default_documents[0].generated_id.is_none());

    let included_documents = collect_documents(
        open_input_values_with_generate_id_and_options(
            vec![
                uri(&hidden_file),
                uri(&nested_hidden_file),
                uri(&visible_file),
            ],
            "body",
            None,
            DiscoveryOptions {
                symlinks: SymlinkMode::Skip,
                hidden: HiddenMode::Include,
            },
        )
        .unwrap(),
    );
    assert_eq!(included_documents.len(), 3);
    assert!(
        included_documents
            .iter()
            .all(|document| document.generated_id.is_some())
    );

    let err = input_err(open_input_values_with_generate_id_and_options(
        vec![uri(&hidden_file), uri(&visible_file)],
        "body",
        None,
        DiscoveryOptions {
            symlinks: SymlinkMode::Skip,
            hidden: HiddenMode::Fail,
        },
    ));
    assert!(err.contains("Hidden path encountered"));
}

#[test]
fn direct_single_hidden_file_bypasses_discovery_policy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".hidden.txt");
    fs::write(&path, "hidden").unwrap();

    let documents = collect_documents(open_input_values(vec![uri(&path)], "body").unwrap());
    assert_eq!(documents.len(), 1);
}

#[test]
fn single_file_glob_preserves_streaming_for_structured_input() {
    let dir = workspace_tempdir();
    let input = dir.path().join("records.ndjson");
    fs::write(&input, "{\"message\":\"hello\"}\n").unwrap();
    let pattern = dir.path().join("*.ndjson");

    let input = open_input_values(vec![uri(&pattern)], "body").unwrap();

    assert!(matches!(input, Input::LocalFileDocuments { .. }));
}

#[test]
fn local_source_count_selects_multi_source_mode_after_discovery() {
    let dir = workspace_tempdir();
    let first = dir.path().join("a.md");
    let second = dir.path().join("b.md");
    fs::write(&first, "a").unwrap();
    fs::write(&second, "b").unwrap();

    let single = open_input_values(vec![uri(&first)], "body").unwrap();
    let multiple = open_input_values(vec![uri(&first), uri(&second)], "body").unwrap();

    assert!(!single.is_multi_source_local());
    assert!(multiple.is_multi_source_local());
}

#[test]
fn shell_expanded_files_are_deduplicated_and_include_origin_metadata() {
    let dir = workspace_tempdir();
    let b = dir.path().join("b.txt");
    let a = dir.path().join("a.txt");
    fs::write(&b, "bravo").unwrap();
    fs::write(&a, "alpha").unwrap();

    let input = open_input_values(vec![uri(&b), uri(&a), uri(&a)], "body").unwrap();
    let values = collect_values(input);

    assert_eq!(values.len(), 2);
    let mut documents = values
        .iter()
        .map(|value| {
            (
                value["origin"]["filename"].as_str().unwrap(),
                value["content"]["body"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    documents.sort();
    assert_eq!(documents, [("a.txt", "alpha"), ("b.txt", "bravo")]);
}

#[test]
fn recursive_glob_imports_regular_files_and_filters_directories() {
    let dir = workspace_tempdir();
    let nested = dir.path().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(dir.path().join("root.md"), "root").unwrap();
    fs::write(nested.join("child.md"), "child").unwrap();

    let pattern = dir
        .path()
        .join("**")
        .join("*.md")
        .to_string_lossy()
        .into_owned();
    let input = open_input_values(vec![UriRef::parse(pattern).unwrap()], "body").unwrap();
    let values = collect_values(input);

    assert_eq!(values.len(), 2);
    let mut documents = values
        .iter()
        .map(|value| {
            (
                value["origin"]["filename"].as_str().unwrap(),
                value["content"]["body"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    documents.sort();
    assert_eq!(documents, [("child.md", "child"), ("root.md", "root")]);
    assert!(
        values
            .iter()
            .all(|value| !value["origin"]["path"].as_str().unwrap().starts_with('/'))
    );
}

#[test]
fn glob_matching_no_regular_files_fails() {
    let dir = tempfile::tempdir().unwrap();
    let pattern = dir
        .path()
        .join("**")
        .join("*.md")
        .to_string_lossy()
        .into_owned();

    let err = input_err(open_input_values(
        vec![UriRef::parse(pattern).unwrap()],
        "body",
    ));

    assert!(err.contains("Glob matched no regular files"));
}

#[test]
fn concrete_missing_and_directory_inputs_are_path_specific_failures() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.md");
    let directory = dir.path().join("docs");
    fs::create_dir(&directory).unwrap();

    let missing_err = input_err(open_input_values(vec![uri(&missing)], "body"));
    assert!(missing_err.contains("File input does not exist"));
    assert!(missing_err.contains("missing.md"));

    let directory_err = input_err(open_input_values(vec![uri(&directory)], "body"));
    assert!(directory_err.contains("File input is not a regular file"));
    assert!(directory_err.contains("docs"));
}
