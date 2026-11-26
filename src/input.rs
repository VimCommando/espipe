mod discovery;
mod documents;
mod metadata;
mod remote;
mod split;
mod stream;
mod workers;
mod yaml;

#[cfg(test)]
mod tests;

pub(crate) use discovery::{DiscoveryOptions, HiddenMode, SymlinkMode};

use self::{
    discovery::{is_batch_import, resolve_file_document_paths_with_options},
    documents::{
        FileDocumentOptions, is_streaming_local_input, open_file_documents_from_paths,
        read_file_document_line, read_local_file_document_line,
    },
    metadata::{FileInputIdentity, OriginMetadata, finalize_file_input_document},
    remote::{fetch_remote_input, fetch_remote_split_input},
    split::{LocalSplitDocuments, SplitInput, open_split_inputs_with_options},
    stream::{
        ToonState, is_unsupported_compressed_input, open_local_file, read_csv_line, read_json_line,
    },
    workers::FileDocumentWorkers,
};
use crate::json_split::SplitPath;
use eyre::{Report, Result, eyre};
use fluent_uri::UriRef;
use serde_json::value::RawValue;
use std::{
    io::{BufReader, Read, Stdin, stdin},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

pub(crate) struct InputOptions {
    pub(crate) content_field: String,
    pub(crate) split: Option<SplitPath>,
    pub(crate) generate_id: Option<bool>,
    pub(crate) discovery: DiscoveryOptions,
}

pub enum Input {
    FileJson {
        source: String,
        reader: Box<BufReader<Box<dyn Read + Send>>>,
        first_record: bool,
        origin: Option<OriginMetadata>,
        file_identity: Option<FileInputIdentity>,
        _temp_file: Option<NamedTempFile>,
    },
    FileCsv {
        source: String,
        reader: Box<csv::Reader<Box<dyn Read + Send>>>,
        origin: Option<OriginMetadata>,
        file_identity: Option<FileInputIdentity>,
        _temp_file: Option<NamedTempFile>,
    },
    FileToon {
        source: String,
        reader: Box<BufReader<Box<dyn Read + Send>>>,
        state: ToonState,
        origin: Option<OriginMetadata>,
        file_identity: Option<FileInputIdentity>,
        _temp_file: Option<NamedTempFile>,
    },
    JsonSplit(SplitInput),
    LocalSplitDocuments(LocalSplitDocuments),
    LocalFileDocuments {
        path: PathBuf,
        generate_id: bool,
        active: Option<Box<Input>>,
        complete: bool,
        evaluated_documents: usize,
    },
    Stdin {
        reader: Box<BufReader<Stdin>>,
    },
    FileDocuments {
        source: String,
        workers: FileDocumentWorkers,
        current_path: Option<PathBuf>,
        documents: Vec<Box<RawValue>>,
        document_index: usize,
        skip_errors: bool,
        bundle_id: Option<String>,
        evaluated_documents: usize,
        file_count: usize,
    },
}

#[derive(Debug)]
pub(crate) struct InputDocument {
    pub(crate) raw: Box<RawValue>,
    pub(crate) generated_id: Option<String>,
}

impl InputDocument {
    fn from_raw(raw: Box<RawValue>) -> Self {
        Self {
            raw,
            generated_id: None,
        }
    }
}

impl std::ops::Deref for InputDocument {
    type Target = RawValue;

    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl Input {
    pub(crate) fn discover_is_multi_source_local(
        uris: &[UriRef<String>],
        discovery_options: DiscoveryOptions,
    ) -> Result<bool> {
        if uris.is_empty()
            || uris.iter().any(|uri| {
                !matches!(
                    uri.scheme().map(|scheme| scheme.as_str()),
                    Some("file") | None
                ) || uri.path().as_str() == "-"
            })
        {
            return Ok(false);
        }

        let (paths, _) = resolve_file_document_paths_with_options(uris, discovery_options)?;
        Ok(paths.len() > 1)
    }

    pub async fn try_new(uris: Vec<UriRef<String>>, options: InputOptions) -> Result<Self> {
        let InputOptions {
            content_field,
            split,
            generate_id,
            discovery,
        } = options;
        validate_content_field(&content_field)?;
        if uris.is_empty() {
            return Err(eyre!("At least one input is required"));
        }
        if let [uri] = uris.as_slice()
            && is_remote(uri)
        {
            let uri = uri.clone();
            return tokio::task::spawn_blocking(move || match split {
                Some(split) => fetch_remote_split_input(uri, split),
                None => fetch_remote_input(uri),
            })
            .await
            .map_err(|err| eyre!("Remote input fetch task failed: {err}"))?;
        }
        match split {
            Some(split) => open_split_inputs_with_options(uris, split, generate_id, discovery),
            None => open_input_values_with_generate_id_and_options(
                uris,
                &content_field,
                generate_id,
                discovery,
            ),
        }
    }

    pub fn read_line(&mut self, line_buffer: &mut String) -> Result<InputDocument> {
        let (raw, origin, identity) = match self {
            Input::FileJson {
                reader,
                first_record,
                origin,
                file_identity,
                ..
            } => {
                let raw = read_json_line(reader, line_buffer, *first_record)?;
                *first_record = false;
                (raw, origin, file_identity)
            }
            Input::FileCsv {
                reader,
                origin,
                file_identity,
                ..
            } => (read_csv_line(reader)?, origin, file_identity),
            Input::FileToon {
                source,
                reader,
                state,
                origin,
                file_identity,
                ..
            } => {
                let raw = state.read(source, reader)?;
                (raw, origin, file_identity)
            }
            Input::JsonSplit(input) => return input.read_document(),
            Input::Stdin { reader } => {
                return read_json_line(reader, line_buffer, false).map(InputDocument::from_raw);
            }
            Input::FileDocuments { .. } => return read_file_document_line(self),
            Input::LocalFileDocuments { .. } => {
                return read_local_file_document_line(self, line_buffer);
            }
            Input::LocalSplitDocuments(input) => return input.read_document(),
        };
        let discriminator = identity
            .as_mut()
            .map(FileInputIdentity::next_record_discriminator);
        finalize_file_input_document(raw, origin.as_ref(), identity.as_mut(), discriminator)
    }

    pub fn read_next(&mut self, line_buffer: &mut String) -> Result<Option<InputDocument>> {
        match self.read_line(line_buffer) {
            Ok(value) => Ok(Some(value)),
            Err(err) if is_end_of_input(&err) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub(crate) fn evaluated_document_count(&self, successful_documents: usize) -> usize {
        match self {
            Input::LocalSplitDocuments(input) => {
                input.evaluated_documents.max(successful_documents)
            }
            Input::LocalFileDocuments {
                evaluated_documents,
                ..
            }
            | Input::FileDocuments {
                evaluated_documents,
                ..
            } => (*evaluated_documents).max(successful_documents),
            _ => successful_documents,
        }
    }

    pub(crate) fn file_count(&self) -> Option<usize> {
        match self {
            Input::FileJson { origin, .. }
            | Input::FileCsv { origin, .. }
            | Input::FileToon { origin, .. } => origin
                .as_ref()
                .filter(|origin| origin.scheme == "file")
                .map(|_| 1),
            Input::JsonSplit(input) => input.file_count(),
            Input::LocalSplitDocuments(input) => Some(input.file_count),
            Input::LocalFileDocuments { .. } => Some(1),
            Input::FileDocuments { file_count, .. } => Some(*file_count),
            Input::Stdin { .. } => None,
        }
    }

    pub(crate) fn is_multi_source_local(&self) -> bool {
        self.file_count().is_some_and(|count| count > 1)
    }
}

impl TryFrom<UriRef<String>> for Input {
    type Error = Report;

    fn try_from(uri: UriRef<String>) -> Result<Self, Self::Error> {
        match uri.scheme().map(|scheme| scheme.as_str()) {
            Some("http" | "https") => fetch_remote_input(uri),
            _ => open_input_values(vec![uri], "body"),
        }
    }
}

impl std::fmt::Display for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Input::FileJson { source, .. } => write!(f, "{source}"),
            Input::FileCsv { source, .. } => write!(f, "{source}"),
            Input::FileToon { source, .. } => write!(f, "{source}"),
            Input::JsonSplit(input) => write!(f, "{}", input.source),
            Input::LocalSplitDocuments(input) => write!(f, "{} split file(s)", input.file_count),
            Input::LocalFileDocuments { path, .. } => write!(f, "{}", path.display()),
            Input::Stdin { .. } => write!(f, "stdin"),
            Input::FileDocuments { source, .. } => write!(f, "{source}"),
        }
    }
}

fn validate_content_field(content_field: &str) -> Result<()> {
    if content_field.is_empty() {
        return Err(eyre!("--content value must not be empty"));
    }
    if content_field.contains('.') {
        return Err(eyre!("--content value must not contain '.'"));
    }
    Ok(())
}

fn open_input_values(uris: Vec<UriRef<String>>, content_field: &str) -> Result<Input> {
    open_input_values_with_generate_id(uris, content_field, None)
}

fn open_input_values_with_generate_id(
    uris: Vec<UriRef<String>>,
    content_field: &str,
    generate_id: Option<bool>,
) -> Result<Input> {
    open_input_values_with_generate_id_and_options(
        uris,
        content_field,
        generate_id,
        DiscoveryOptions::default(),
    )
}

fn open_input_values_with_generate_id_and_options(
    uris: Vec<UriRef<String>>,
    content_field: &str,
    generate_id: Option<bool>,
    discovery_options: DiscoveryOptions,
) -> Result<Input> {
    if let [uri] = uris.as_slice()
        && is_remote(uri)
    {
        return fetch_remote_input(uri.clone());
    }
    validate_input_schemes(&uris, true)?;
    if is_stdin(&uris) {
        return Ok(Input::Stdin {
            reader: Box::new(BufReader::new(stdin())),
        });
    }

    let skip_errors = is_batch_import(&uris);
    let (paths, origins) = resolve_file_document_paths_with_options(&uris, discovery_options)?;
    let generate_id = effective_generate_id(generate_id, paths.len());
    let options = FileDocumentOptions {
        content_field,
        generate_id,
        skip_errors,
    };
    if let [path] = paths.as_slice() {
        if !skip_errors {
            return open_single_local_input(&uris[0], paths, origins, options);
        }
        if is_streaming_local_input(path) {
            return Ok(Input::LocalFileDocuments {
                path: paths.into_iter().next().unwrap(),
                generate_id,
                active: None,
                complete: false,
                evaluated_documents: 0,
            });
        }
    }
    open_file_documents_from_paths(paths, origins, options)
}

fn open_single_local_input(
    uri: &UriRef<String>,
    paths: Vec<PathBuf>,
    origins: Vec<OriginMetadata>,
    options: FileDocumentOptions<'_>,
) -> Result<Input> {
    if is_streaming_local_input(&paths[0]) {
        return open_local_file(paths.into_iter().next().unwrap(), options.generate_id);
    }
    let path = uri.path().as_str();
    if is_unsupported_compressed_input(path) {
        return Err(eyre!("Unsupported compressed input format: {path}"));
    }
    open_file_documents_from_paths(paths, origins, options)
}

fn validate_input_schemes(uris: &[UriRef<String>], allow_remote: bool) -> Result<()> {
    for uri in uris {
        match uri.scheme().map(|scheme| scheme.as_str()) {
            Some("file") | None => {}
            Some("http" | "https") if allow_remote => {
                return Err(eyre!("Remote inputs cannot be combined with file imports"));
            }
            Some(scheme) => return Err(eyre!("Unsupported input scheme: {scheme}")),
        }
    }
    Ok(())
}

fn is_remote(uri: &UriRef<String>) -> bool {
    matches!(
        uri.scheme().map(|scheme| scheme.as_str()),
        Some("http" | "https")
    )
}

fn is_stdin(uris: &[UriRef<String>]) -> bool {
    matches!(uris, [uri] if uri.scheme().is_none() && uri.path().as_str() == "-")
}

fn effective_generate_id(mode: Option<bool>, source_count: usize) -> bool {
    mode.unwrap_or(source_count > 1)
}

fn log_skipped_file(path: &Path, error: impl std::fmt::Display) {
    let error = error.to_string();
    let path_prefix = format!("{}: ", path.display());
    if error.starts_with(&path_prefix) {
        log::warn!("skipping file {error}");
    } else {
        log::warn!("skipping file {}: {error}", path.display());
    }
}

fn is_end_of_input(err: &eyre::Report) -> bool {
    matches!(
        err.to_string().as_str(),
        "No JSON record"
            | "No CSV record"
            | "No file document"
            | "No Toon document"
            | "No split document"
    )
}
