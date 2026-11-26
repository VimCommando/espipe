use super::{
    Input,
    metadata::origin_from_uri,
    split::{SplitMetadata, open_json_split},
    stream::{
        InputKind, StreamSource, ensure_json_opening, has_path_suffix, input_kind_from_path,
        open_stream,
    },
};
use crate::json_split::SplitPath;
use eyre::{Result, eyre};
use fluent_uri::UriRef;
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT, CONTENT_TYPE},
};
use serde_json::value::RawValue;
use std::{
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom, Write},
    time::Duration,
};
use tempfile::{Builder, NamedTempFile};

pub(super) const REMOTE_NDJSON_ERROR: &str =
    "JSON payload does not look like required NDJSON input format.";
const REMOTE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) fn fetch_remote_input(uri: UriRef<String>) -> Result<Input> {
    fetch_remote_input_with_client(uri, &remote_client()?)
}

pub(super) fn fetch_remote_split_input(uri: UriRef<String>, split: SplitPath) -> Result<Input> {
    fetch_remote_input_with_client_and_split(uri, &remote_client()?, Some(split))
}

fn remote_client() -> Result<Client> {
    Ok(Client::builder()
        .connect_timeout(REMOTE_CONNECT_TIMEOUT)
        .timeout(REMOTE_REQUEST_TIMEOUT)
        .build()?)
}

pub(super) fn fetch_remote_input_with_client(
    uri: UriRef<String>,
    client: &Client,
) -> Result<Input> {
    fetch_remote_input_with_client_and_split(uri, client, None)
}

pub(super) fn fetch_remote_input_with_client_and_split(
    uri: UriRef<String>,
    client: &Client,
    split: Option<SplitPath>,
) -> Result<Input> {
    let mut response = client
        .get(uri.as_str())
        .header(
            ACCEPT,
            "text/csv, application/x-ndjson, application/ndjson, application/json, application/toon, application/x-toon, text/toon",
        )
        .send()?;

    if !response.status().is_success() {
        return Err(eyre!(
            "Remote fetch failed with HTTP status {}",
            response.status()
        ));
    }

    let kind = remote_input_kind(&uri, &response)?;
    if split.is_some() && !matches!(kind, InputKind::Json) {
        return Err(eyre!("--split requires a JSON input source"));
    }
    let mut temp_file = save_response(&mut response, kind)?;

    if kind == InputKind::Json && split.is_none() {
        validate_ndjson_file(temp_file.as_file_mut())?;
    }

    let reader_file = temp_file.reopen()?;
    let source = uri.to_string();
    let origin = Some(origin_from_uri(&uri));

    if let Some(split) = split {
        return open_json_split(
            Box::new(reader_file),
            source,
            split,
            SplitMetadata {
                origin,
                file_identity: None,
                _temp_file: Some(temp_file),
            },
        );
    }

    open_stream(
        kind,
        Box::new(reader_file),
        StreamSource {
            source,
            origin,
            file_identity: None,
            temp_file: Some(temp_file),
        },
    )
}

fn save_response(response: &mut Response, kind: InputKind) -> Result<NamedTempFile> {
    let suffix = match kind {
        InputKind::Csv => ".csv",
        InputKind::Ndjson => ".ndjson",
        InputKind::Json => ".json",
        InputKind::Toon => ".toon",
        InputKind::FileDocument => return Err(eyre!("Unsupported remote input format")),
    };
    let mut temp_file = Builder::new().suffix(suffix).tempfile()?;
    std::io::copy(response, temp_file.as_file_mut())?;
    temp_file.as_file_mut().flush()?;
    Ok(temp_file)
}

fn remote_input_kind(uri: &UriRef<String>, response: &Response) -> Result<InputKind> {
    if has_path_suffix(uri.path().as_str(), ".gz") {
        return Err(eyre!(
            "Unsupported remote gzip input format: {}",
            uri.path()
        ));
    }
    if let Some(kind) = input_kind_from_path(uri.path().as_str()) {
        return Ok(kind);
    }

    let Some(content_type) = response.headers().get(CONTENT_TYPE) else {
        return Err(eyre!("Unsupported remote input format"));
    };
    let content_type = content_type.to_str()?.to_ascii_lowercase();

    input_kind_from_content_type(&content_type)
        .ok_or_else(|| eyre!("Unsupported remote input format"))
}

fn input_kind_from_content_type(content_type: &str) -> Option<InputKind> {
    let formats: &[(InputKind, &[&str])] = &[
        (InputKind::Csv, &["text/csv", "application/csv"]),
        (
            InputKind::Ndjson,
            &["application/x-ndjson", "application/ndjson"],
        ),
        (InputKind::Json, &["application/json"]),
        (
            InputKind::Toon,
            &["application/toon", "application/x-toon", "text/toon"],
        ),
    ];
    for &(kind, media_types) in formats {
        let matching_media_type = media_types
            .iter()
            .any(|media_type| content_type.contains(media_type));
        let structured_json = kind == InputKind::Json && content_type.ends_with("+json");
        if matching_media_type || structured_json {
            return Some(kind);
        }
    }
    None
}

pub(super) fn validate_ndjson_file(file: &mut File) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut reader = BufReader::new(&mut *file);
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }

        let raw: Box<RawValue> =
            serde_json::from_str(&line).map_err(|_| eyre!(REMOTE_NDJSON_ERROR))?;
        ensure_json_opening(raw.get(), REMOTE_NDJSON_ERROR)?;
    }

    file.seek(SeekFrom::Start(0))?;
    Ok(())
}
