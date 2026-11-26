use super::collect_documents;
use super::workspace_tempdir;
use crate::input::InputDocument;
use crate::input::documents::{FileDocumentOptions, open_file_documents_from_paths_with_converter};
use crate::input::metadata::OriginMetadata;
use crate::input::metadata::origin_from_local_path;
use crate::input::workers::FileDocumentConverter;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Barrier;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

#[test]
fn multi_source_file_workers_emit_as_completed_with_stable_ids() {
    let dir = workspace_tempdir();
    let (paths, origins) = conversion_sources(dir.path(), 6);
    let repeat_paths = paths.clone();
    let repeat_origins = origins.clone();
    let (converter, maximum_active) = timed_converter();

    let documents = collect_documents(
        open_file_documents_from_paths_with_converter(
            paths,
            origins,
            FileDocumentOptions {
                content_field: "body",
                generate_id: true,
                skip_errors: false,
            },
            Some((3, converter)),
        )
        .unwrap(),
    );

    assert!(maximum_active.load(Ordering::SeqCst) >= 2);
    let filenames: Vec<_> = documents
        .iter()
        .map(|document| {
            serde_json::from_str::<serde_json::Value>(document.get()).unwrap()["filename"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_ne!(filenames[0], "0.txt");
    let mut sorted_filenames = filenames.clone();
    sorted_filenames.sort();
    assert_eq!(
        sorted_filenames,
        ["0.txt", "1.txt", "2.txt", "3.txt", "4.txt", "5.txt"]
    );
    assert!(
        documents
            .iter()
            .all(|document| document.generated_id.is_some())
    );

    let repeat_converter = Arc::new(|path: &Path, _: &str, _: Option<&OriginMetadata>| {
        let filename = path.file_name().unwrap().to_string_lossy();
        let raw =
            serde_json::value::RawValue::from_string(format!(r#"{{"filename":"{filename}"}}"#))?;
        Ok(vec![raw])
    }) as FileDocumentConverter;
    let repeated = collect_documents(
        open_file_documents_from_paths_with_converter(
            repeat_paths,
            repeat_origins,
            FileDocumentOptions {
                content_field: "body",
                generate_id: true,
                skip_errors: false,
            },
            Some((3, repeat_converter)),
        )
        .unwrap(),
    );
    assert_eq!(ids_by_filename(&documents), ids_by_filename(&repeated));
}

fn conversion_sources(dir: &Path, count: usize) -> (Vec<PathBuf>, Vec<OriginMetadata>) {
    (0..count)
        .map(|index| {
            let path = dir.join(format!("{index}.txt"));
            fs::write(&path, format!("document {index}")).unwrap();
            let origin = origin_from_local_path(&path);
            (path, origin)
        })
        .unzip()
}

fn ids_by_filename(documents: &[InputDocument]) -> BTreeMap<String, String> {
    documents
        .iter()
        .map(|document| {
            let value: serde_json::Value = serde_json::from_str(document.get()).unwrap();
            (
                value["filename"].as_str().unwrap().to_string(),
                document.generated_id.clone().unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>()
}

fn timed_converter() -> (FileDocumentConverter, Arc<AtomicUsize>) {
    let active = Arc::new(AtomicUsize::new(0));
    let maximum_active = Arc::new(AtomicUsize::new(0));
    let converter = {
        let active = Arc::clone(&active);
        let maximum_active = Arc::clone(&maximum_active);
        Arc::new(move |path: &Path, _: &str, _: Option<&OriginMetadata>| {
            let active_now = active.fetch_add(1, Ordering::SeqCst) + 1;
            maximum_active.fetch_max(active_now, Ordering::SeqCst);
            let filename = path.file_name().unwrap().to_string_lossy().into_owned();
            if filename == "0.txt" {
                thread::sleep(Duration::from_millis(60));
            } else {
                thread::sleep(Duration::from_millis(10));
            }
            active.fetch_sub(1, Ordering::SeqCst);
            let raw = serde_json::value::RawValue::from_string(format!(
                r#"{{"filename":"{filename}"}}"#
            ))?;
            Ok(vec![raw])
        }) as FileDocumentConverter
    };

    (converter, maximum_active)
}

#[test]
fn multi_source_file_workers_recover_from_failure_during_active_conversion() {
    let dir = workspace_tempdir();
    let (paths, origins) = conversion_sources(dir.path(), 3);
    let first_jobs_started = Arc::new(Barrier::new(2));
    let active = Arc::new(AtomicUsize::new(0));
    let failure_saw_parallel_work = Arc::new(AtomicBool::new(false));
    let converter = {
        let first_jobs_started = Arc::clone(&first_jobs_started);
        let active = Arc::clone(&active);
        let failure_saw_parallel_work = Arc::clone(&failure_saw_parallel_work);
        Arc::new(move |path: &Path, _: &str, _: Option<&OriginMetadata>| {
            active.fetch_add(1, Ordering::SeqCst);
            let filename = path.file_name().unwrap().to_string_lossy().into_owned();
            if filename == "0.txt" || filename == "1.txt" {
                first_jobs_started.wait();
            }
            if filename == "0.txt" {
                failure_saw_parallel_work
                    .store(active.load(Ordering::SeqCst) >= 2, Ordering::SeqCst);
                active.fetch_sub(1, Ordering::SeqCst);
                return Err(eyre::eyre!("forced conversion failure"));
            }
            if filename == "1.txt" {
                thread::sleep(Duration::from_millis(30));
            }
            active.fetch_sub(1, Ordering::SeqCst);
            let raw = serde_json::value::RawValue::from_string(format!(
                r#"{{"filename":"{filename}"}}"#
            ))?;
            Ok(vec![raw])
        }) as FileDocumentConverter
    };

    let documents = collect_documents(
        open_file_documents_from_paths_with_converter(
            paths,
            origins,
            FileDocumentOptions {
                content_field: "body",
                generate_id: false,
                skip_errors: true,
            },
            Some((2, converter)),
        )
        .unwrap(),
    );

    assert!(failure_saw_parallel_work.load(Ordering::SeqCst));
    let mut filenames = documents
        .iter()
        .map(|document| {
            serde_json::from_str::<serde_json::Value>(document.get()).unwrap()["filename"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();
    filenames.sort();
    assert_eq!(filenames, ["1.txt", "2.txt"]);
}
