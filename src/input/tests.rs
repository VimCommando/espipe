mod anydoc;
mod discovery;
mod documents;
mod identity;
mod remote;
mod split;
mod streams;
mod workers;

use super::{Input, InputDocument};
use base64::Engine as _;
use flate2::{Compression, write::GzEncoder};
use fluent_uri::UriRef;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn uri(path: &Path) -> UriRef<String> {
    UriRef::parse(path.to_string_lossy().into_owned()).unwrap()
}

pub(super) fn collect_values(input: Input) -> Vec<serde_json::Value> {
    collect_documents(input)
        .into_iter()
        .map(|document| serde_json::from_str(document.get()).unwrap())
        .collect()
}

pub(super) fn collect_documents(mut input: Input) -> Vec<InputDocument> {
    let mut documents = Vec::new();
    let mut line = String::new();
    while let Ok(value) = input.read_line(&mut line) {
        documents.push(value);
        line.clear();
    }
    documents
}

pub(super) fn input_err(result: eyre::Result<Input>) -> String {
    match result {
        Ok(_) => panic!("expected input construction to fail"),
        Err(err) => err.to_string(),
    }
}

pub(super) fn read_err(result: eyre::Result<Input>) -> String {
    let mut input = result.unwrap();
    let mut line = String::new();
    input.read_line(&mut line).unwrap_err().to_string()
}

pub(super) fn temp_path(suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("espipe-input-{nanos}.{suffix}"))
}

pub(super) fn workspace_tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("espipe-test-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
}

pub(super) fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub(super) fn write_base64_fixture(source: &str, path: &Path) {
    let encoded = fs::read_to_string(fixture_path(source)).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .unwrap();
    fs::write(path, bytes).unwrap();
}

pub(super) fn write_gzip(path: &Path, contents: &str) {
    let file = fs::File::create(path).unwrap();
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(contents.as_bytes()).unwrap();
    encoder.finish().unwrap();
}
