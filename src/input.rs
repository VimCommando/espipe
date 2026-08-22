use crate::json_split::{SplitDocument, SplitEvent, SplitPath, start_split_reader};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use clap::ValueEnum;
use eyre::{Report, Result, eyre};
use flate2::read::GzDecoder;
use fluent_uri::UriRef;
use glob::glob;
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT, CONTENT_TYPE},
};
use serde::de::{EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde_json::{Map, Value, value::RawValue};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::OsStr,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Stdin, Write, stdin},
    path::{Component, Path, PathBuf},
    sync::mpsc::Receiver,
    time::Duration,
};
use tempfile::{Builder, NamedTempFile};

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
        pending: String,
        document_index: usize,
        buffered_rows: Vec<Value>,
        eof: bool,
        origin: Option<OriginMetadata>,
        file_identity: Option<FileInputIdentity>,
        _temp_file: Option<NamedTempFile>,
    },
    JsonSplit {
        source: String,
        receiver: Receiver<SplitEvent>,
        pending_documents: VecDeque<SplitDocument>,
        finished: bool,
        origin: Option<OriginMetadata>,
        file_identity: Option<FileInputIdentity>,
        _temp_file: Option<NamedTempFile>,
    },
    LocalSplitDocuments {
        paths: Vec<PathBuf>,
        origins: Vec<OriginMetadata>,
        path_index: usize,
        split: SplitPath,
        generate_id: bool,
        active: Option<ActiveLocalSplit>,
    },
    Stdin {
        reader: Box<BufReader<Stdin>>,
    },
    FileDocuments {
        source: String,
        paths: Vec<PathBuf>,
        origins: Vec<OriginMetadata>,
        path_index: usize,
        documents: Vec<Box<RawValue>>,
        document_index: usize,
        content_field: String,
        generate_id: bool,
        bundle_id: String,
    },
}

#[derive(Debug)]
pub(crate) struct InputDocument {
    pub(crate) raw: Box<RawValue>,
    pub(crate) generated_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum SymlinkMode {
    Follow,
    Fail,
    Skip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum HiddenMode {
    Include,
    Fail,
    Skip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DiscoveryOptions {
    pub(crate) symlinks: SymlinkMode,
    pub(crate) hidden: HiddenMode,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            symlinks: SymlinkMode::Skip,
            hidden: HiddenMode::Skip,
        }
    }
}

pub(crate) struct ActiveLocalSplit {
    source: String,
    receiver: Receiver<SplitEvent>,
    pending_documents: VecDeque<SplitDocument>,
    origin: OriginMetadata,
    file_identity: FileInputIdentity,
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

#[derive(Clone, Debug)]
pub(crate) struct OriginMetadata {
    scheme: String,
    authority: Option<String>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
    filename: String,
}

#[derive(Clone, Debug)]
pub(crate) struct FileInputIdentity {
    path: PathBuf,
    bundle_id: String,
    generate_id: bool,
    document_index: usize,
}

impl FileInputIdentity {
    fn new(path: &Path, generate_id: bool) -> Result<Self> {
        Ok(Self {
            path: normalize_local_path(path)?,
            bundle_id: if generate_id {
                bundle_identifier()?
            } else {
                String::new()
            },
            generate_id,
            document_index: 0,
        })
    }

    fn next_record_discriminator(&mut self) -> DocumentDiscriminator {
        let index = self.document_index;
        self.document_index += 1;
        DocumentDiscriminator::Record(index)
    }
}

#[derive(Clone, Debug)]
enum DocumentDiscriminator {
    Record(usize),
    SplitKey(String),
    SplitIndex(usize),
}

impl DocumentDiscriminator {
    fn encode(&self) -> String {
        match self {
            Self::Record(index) => format!("record:{index}"),
            Self::SplitKey(key) => format!("key:{key}"),
            Self::SplitIndex(index) => format!("index:{index}"),
        }
    }
}

type CsvRecord = std::collections::HashMap<String, String>;
const REMOTE_NDJSON_ERROR: &str = "JSON payload does not look like required NDJSON input format.";
const JSON_LINE_OPENING_ERROR: &str = "Each record must be a JSON object starting with '{'";
const REMOTE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputKind {
    Csv,
    Ndjson,
    Json,
    Toon,
    FileDocument,
}

impl Input {
    pub async fn try_new(
        uris: Vec<UriRef<String>>,
        content_field: String,
        split: Option<SplitPath>,
        generate_id: Option<bool>,
        discovery_options: DiscoveryOptions,
    ) -> Result<Self> {
        validate_content_field(&content_field)?;
        if uris.is_empty() {
            return Err(eyre!("At least one input is required"));
        }
        if let Some(split) = split {
            if uris.len() == 1
                && matches!(
                    uris[0].scheme().map(|scheme| scheme.as_str()),
                    Some("http" | "https")
                )
            {
                let uri = uris.into_iter().next().unwrap();
                return tokio::task::spawn_blocking(move || fetch_remote_split_input(uri, split))
                    .await
                    .map_err(|err| eyre!("Remote input fetch task failed: {err}"))?;
            }
            return open_split_inputs_with_options(uris, split, generate_id, discovery_options);
        }
        if uris.len() == 1 {
            let uri = uris.into_iter().next().unwrap();
            return match uri.scheme().map(|scheme| scheme.as_str()) {
                Some("http" | "https") => {
                    tokio::task::spawn_blocking(move || fetch_remote_input(uri))
                        .await
                        .map_err(|err| eyre!("Remote input fetch task failed: {err}"))?
                }
                _ => open_input_values_with_generate_id_and_options(
                    vec![uri],
                    &content_field,
                    generate_id,
                    discovery_options,
                ),
            };
        }
        open_input_values_with_generate_id_and_options(
            uris,
            &content_field,
            generate_id,
            discovery_options,
        )
    }

    pub fn read_line(&mut self, line_buffer: &mut String) -> Result<InputDocument> {
        match self {
            Input::FileJson {
                reader,
                first_record,
                origin,
                file_identity,
                ..
            } => {
                let raw = read_json_line(reader, line_buffer, *first_record)?;
                *first_record = false;
                let discriminator = file_identity
                    .as_mut()
                    .map(FileInputIdentity::next_record_discriminator);
                finalize_file_input_document(
                    raw,
                    origin.as_ref(),
                    file_identity.as_mut(),
                    discriminator,
                )
            }
            Input::FileCsv {
                reader,
                origin,
                file_identity,
                ..
            } => {
                let raw = read_csv_line(reader)?;
                let discriminator = file_identity
                    .as_mut()
                    .map(FileInputIdentity::next_record_discriminator);
                finalize_file_input_document(
                    raw,
                    origin.as_ref(),
                    file_identity.as_mut(),
                    discriminator,
                )
            }
            Input::FileToon {
                source,
                reader,
                pending,
                document_index,
                buffered_rows,
                eof,
                origin,
                file_identity,
                ..
            } => {
                let raw = read_toon_document(
                    source,
                    reader,
                    pending,
                    document_index,
                    buffered_rows,
                    eof,
                )?;
                let discriminator = file_identity
                    .as_mut()
                    .map(FileInputIdentity::next_record_discriminator);
                finalize_file_input_document(
                    raw,
                    origin.as_ref(),
                    file_identity.as_mut(),
                    discriminator,
                )
            }
            Input::JsonSplit {
                source,
                receiver,
                pending_documents,
                finished,
                origin,
                file_identity,
                ..
            } => {
                if *finished {
                    return Err(eyre!("No split document"));
                }
                loop {
                    if let Some(document) = pending_documents.pop_front() {
                        return finalize_split_document(
                            document,
                            origin.as_ref(),
                            file_identity.as_mut(),
                        );
                    }
                    match receiver.recv() {
                        Ok(SplitEvent::Documents(documents)) => {
                            pending_documents.extend(documents);
                        }
                        Ok(SplitEvent::Failure(error)) => {
                            *finished = true;
                            return Err(eyre!(error));
                        }
                        Ok(SplitEvent::Complete) => {
                            *finished = true;
                            return Err(eyre!("No split document"));
                        }
                        Err(_) => {
                            *finished = true;
                            return Err(eyre!("{source}: JSON split parser stopped unexpectedly"));
                        }
                    }
                }
            }
            Input::Stdin { reader, .. } => {
                read_json_line(reader, line_buffer, false).map(InputDocument::from_raw)
            }
            Input::FileDocuments { .. } => read_file_document_line(self),
            Input::LocalSplitDocuments { .. } => read_local_split_line(self),
        }
    }

    pub fn read_next(&mut self, line_buffer: &mut String) -> Result<Option<InputDocument>> {
        match self.read_line(line_buffer) {
            Ok(value) => Ok(Some(value)),
            Err(err) if is_end_of_input(&err) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

fn finalize_file_input_document(
    raw: Box<RawValue>,
    origin: Option<&OriginMetadata>,
    file_identity: Option<&mut FileInputIdentity>,
    discriminator: Option<DocumentDiscriminator>,
) -> Result<InputDocument> {
    let raw = add_origin_to_raw(raw, origin)?;
    let generated_id = match (file_identity, discriminator) {
        (Some(identity), Some(discriminator)) if identity.generate_id => {
            let has_explicit_id = serde_json::from_str::<Value>(raw.get())
                .ok()
                .and_then(|value| value.as_object().map(|object| object.contains_key("_id")))
                .unwrap_or(false);
            (!has_explicit_id)
                .then(|| file_document_id(&identity.bundle_id, &identity.path, discriminator))
                .transpose()?
        }
        _ => None,
    };
    Ok(InputDocument { raw, generated_id })
}

fn finalize_split_document(
    document: SplitDocument,
    origin: Option<&OriginMetadata>,
    file_identity: Option<&mut FileInputIdentity>,
) -> Result<InputDocument> {
    let discriminator = match document.discriminator {
        crate::json_split::SplitDiscriminator::MapKey(key) => DocumentDiscriminator::SplitKey(key),
        crate::json_split::SplitDiscriminator::ArrayIndex(index) => {
            DocumentDiscriminator::SplitIndex(index)
        }
    };
    finalize_file_input_document(document.raw, origin, file_identity, Some(discriminator))
}

fn read_local_split_line(input: &mut Input) -> Result<InputDocument> {
    let Input::LocalSplitDocuments {
        paths,
        origins,
        path_index,
        split,
        generate_id,
        active,
    } = input
    else {
        return Err(eyre!("Input is not a local split import"));
    };

    loop {
        if active.is_none() {
            let Some(path) = paths.get(*path_index) else {
                return Err(eyre!("No split document"));
            };
            let origin = origins
                .get(*path_index)
                .cloned()
                .ok_or_else(|| eyre!("Local split origin cursor is invalid"))?;
            *path_index += 1;
            let source = path.display().to_string();
            let file = File::open(path)?;
            let receiver =
                start_split_reader(local_file_reader(file, path), source.clone(), split.clone())?;
            *active = Some(ActiveLocalSplit {
                source,
                receiver,
                pending_documents: VecDeque::new(),
                origin,
                file_identity: FileInputIdentity::new(path, *generate_id)?,
            });
        }

        if let Some(document) = active
            .as_mut()
            .and_then(|state| state.pending_documents.pop_front())
        {
            let state = active.as_mut().expect("active split state disappeared");
            return finalize_split_document(
                document,
                Some(&state.origin),
                Some(&mut state.file_identity),
            );
        }

        let event = active
            .as_mut()
            .expect("active split state disappeared")
            .receiver
            .recv();
        match event {
            Ok(SplitEvent::Documents(documents)) => {
                active
                    .as_mut()
                    .expect("active split state disappeared")
                    .pending_documents
                    .extend(documents);
            }
            Ok(SplitEvent::Failure(error)) => {
                let source = active
                    .as_ref()
                    .map(|state| state.source.clone())
                    .unwrap_or_else(|| "local split".to_string());
                *active = None;
                return Err(eyre!("{source}: {error}"));
            }
            Ok(SplitEvent::Complete) => {
                *active = None;
            }
            Err(_) => {
                let source = active
                    .as_ref()
                    .map(|state| state.source.clone())
                    .unwrap_or_else(|| "local split".to_string());
                *active = None;
                return Err(eyre!("{source}: JSON split parser stopped unexpectedly"));
            }
        }
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
            Input::JsonSplit { source, .. } => write!(f, "{source}"),
            Input::LocalSplitDocuments { paths, .. } => {
                write!(f, "{} split file(s)", paths.len())
            }
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
    for uri in &uris {
        match uri.scheme().map(|scheme| scheme.as_str()) {
            Some("http" | "https") if uris.len() == 1 => return fetch_remote_input(uri.clone()),
            Some("http" | "https") => {
                return Err(eyre!("Remote inputs cannot be combined with file imports"));
            }
            Some("file") | None => {}
            Some(scheme) => return Err(eyre!("Unsupported input scheme: {scheme}")),
        }
    }

    if uris.len() == 1 && uris[0].scheme().is_none() && uris[0].path().as_str() == "-" {
        return Ok(Input::Stdin {
            reader: Box::new(BufReader::new(stdin())),
        });
    }

    let (paths, origins) =
        resolve_file_document_paths_with_options(uris.clone(), discovery_options)?;
    let generate_id = effective_generate_id(generate_id, paths.len());

    if paths.len() == 1 && uris.len() == 1 {
        let uri = uris.into_iter().next().unwrap();
        let path_str = uri.path().as_str();
        let path = paths.into_iter().next().unwrap();
        if let Ok(kind) = local_input_kind(&path) {
            match kind {
                InputKind::Csv | InputKind::Ndjson | InputKind::Toon => {
                    return open_local_file(path, generate_id);
                }
                InputKind::Json if !should_use_file_document(&path) => {
                    return open_local_file(path, generate_id);
                }
                InputKind::Json | InputKind::FileDocument => {}
            }
        }
        if is_unsupported_compressed_input(path_str) {
            return Err(eyre!("Unsupported compressed input format: {path_str}"));
        }
        return open_file_documents_from_paths(vec![path], origins, content_field, generate_id);
    }

    open_file_documents_from_paths(paths, origins, content_field, generate_id)
}

fn effective_generate_id(mode: Option<bool>, source_count: usize) -> bool {
    mode.unwrap_or(source_count > 1)
}

#[cfg(test)]
fn open_split_inputs(
    uris: Vec<UriRef<String>>,
    split: SplitPath,
    generate_id: Option<bool>,
) -> Result<Input> {
    open_split_inputs_with_options(uris, split, generate_id, DiscoveryOptions::default())
}

fn open_split_inputs_with_options(
    uris: Vec<UriRef<String>>,
    split: SplitPath,
    generate_id: Option<bool>,
    discovery_options: DiscoveryOptions,
) -> Result<Input> {
    for uri in &uris {
        match uri.scheme().map(|scheme| scheme.as_str()) {
            Some("file") | None => {}
            Some(scheme) => return Err(eyre!("Unsupported input scheme: {scheme}")),
        }
    }

    if uris.len() == 1 && uris[0].scheme().is_none() && uris[0].path().as_str() == "-" {
        return open_json_split(
            Box::new(stdin()),
            "stdin".to_string(),
            split,
            None,
            None,
            None,
        );
    }

    let (paths, origins) = resolve_file_document_paths_with_options(uris, discovery_options)?;
    for path in &paths {
        if !matches!(local_input_kind(path)?, InputKind::Json) {
            return Err(eyre!("--split requires a JSON input source"));
        }
    }
    let effective_generate_id = effective_generate_id(generate_id, paths.len());
    if paths.len() == 1 {
        let path = paths.into_iter().next().unwrap();
        let origin = origins.into_iter().next();
        return open_local_split_input(path, origin, split, effective_generate_id);
    }

    Ok(Input::LocalSplitDocuments {
        paths,
        origins,
        path_index: 0,
        split,
        generate_id: effective_generate_id,
        active: None,
    })
}

fn open_json_split(
    reader: Box<dyn Read + Send>,
    source: String,
    split: SplitPath,
    origin: Option<OriginMetadata>,
    file_identity: Option<FileInputIdentity>,
    temp_file: Option<NamedTempFile>,
) -> Result<Input> {
    let receiver = start_split_reader(reader, source.clone(), split)?;
    Ok(Input::JsonSplit {
        source,
        receiver,
        pending_documents: VecDeque::new(),
        finished: false,
        origin,
        file_identity,
        _temp_file: temp_file,
    })
}

fn open_local_split_input(
    path: PathBuf,
    origin: Option<OriginMetadata>,
    split: SplitPath,
    generate_id: bool,
) -> Result<Input> {
    let file_identity = Some(FileInputIdentity::new(&path, generate_id)?);
    let source = path.display().to_string();
    let file = File::open(&path)?;
    open_json_split(
        local_file_reader(file, &path),
        source,
        split,
        origin,
        file_identity,
        None,
    )
}

fn read_json_line<R: BufRead>(
    reader: &mut R,
    line_buffer: &mut String,
    first_record: bool,
) -> Result<Box<RawValue>> {
    reader.read_line(line_buffer)?;
    if line_buffer.is_empty() {
        return Err(eyre!("No JSON record"));
    }
    if first_record && line_buffer.trim() == "{" {
        let mut rest = String::new();
        reader.read_to_string(&mut rest)?;
        line_buffer.push_str(&rest);
        let raw: Box<RawValue> =
            serde_json::from_str(line_buffer).map_err(|e| eyre!("Error parsing JSON: {e}"))?;
        ensure_json_opening(raw.get(), JSON_LINE_OPENING_ERROR)?;
        return Ok(raw);
    }
    let raw: Box<RawValue> =
        serde_json::from_str(line_buffer).map_err(|e| eyre!("Error parsing JSON: {e}"))?;
    ensure_json_opening(raw.get(), JSON_LINE_OPENING_ERROR)?;
    Ok(raw)
}

fn read_csv_line(reader: &mut csv::Reader<Box<dyn Read + Send>>) -> Result<Box<RawValue>> {
    match reader.deserialize::<CsvRecord>().next() {
        Some(Ok(record)) => {
            let json = serde_json::to_string(&record)?;
            serde_json::value::RawValue::from_string(json).map_err(Into::into)
        }
        Some(Err(err)) => Err(err.into()),
        None => Err(eyre!("No CSV record")),
    }
}

fn read_toon_document<R: BufRead>(
    source: &str,
    reader: &mut R,
    pending: &mut String,
    document_index: &mut usize,
    buffered_rows: &mut Vec<Value>,
    eof: &mut bool,
) -> Result<Box<RawValue>> {
    if let Some(row) = buffered_rows.pop() {
        return toon_row_value_to_raw(source, *document_index, row);
    }

    if *eof {
        return Err(eyre!("No Toon document"));
    }

    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            *eof = true;
            if pending.trim().is_empty() {
                return Err(eyre!("No Toon document"));
            }
            *document_index += 1;
            let raw = decode_toon_documents(source, *document_index, pending, buffered_rows)?;
            pending.clear();
            return Ok(raw);
        }

        if line.trim() == "---" {
            if pending.trim().is_empty() {
                continue;
            }
            *document_index += 1;
            let raw = decode_toon_documents(source, *document_index, pending, buffered_rows)?;
            pending.clear();
            return Ok(raw);
        }

        pending.push_str(&line);
    }
}

fn open_local_file(path: PathBuf, generate_id: bool) -> Result<Input> {
    let source = path.display().to_string();
    let file = File::open(&path)?;
    match local_input_kind(&path)? {
        InputKind::Csv => {
            let file_identity = FileInputIdentity::new(&path, generate_id)?;
            let origin = Some(origin_from_local_path(&file_identity.path));
            Ok(Input::FileCsv {
                source,
                reader: Box::new(
                    csv::ReaderBuilder::new()
                        .has_headers(true)
                        .from_reader(local_file_reader(file, &path)),
                ),
                origin,
                file_identity: Some(file_identity),
                _temp_file: None,
            })
        }
        InputKind::Ndjson | InputKind::Json => {
            let file_identity = FileInputIdentity::new(&path, generate_id)?;
            let origin = Some(origin_from_local_path(&file_identity.path));
            Ok(Input::FileJson {
                source,
                reader: Box::new(BufReader::new(local_file_reader(file, &path))),
                first_record: true,
                origin,
                file_identity: Some(file_identity),
                _temp_file: None,
            })
        }
        InputKind::Toon => {
            let file_identity = FileInputIdentity::new(&path, generate_id)?;
            let origin = Some(origin_from_local_path(&file_identity.path));
            Ok(Input::FileToon {
                source,
                reader: Box::new(BufReader::new(local_file_reader(file, &path))),
                pending: String::new(),
                document_index: 0,
                buffered_rows: Vec::new(),
                eof: false,
                origin,
                file_identity: Some(file_identity),
                _temp_file: None,
            })
        }
        InputKind::FileDocument => open_file_documents(
            vec![UriRef::parse(source).map_err(|err| eyre!("Invalid local file URI: {err:?}"))?],
            "body",
            generate_id,
        ),
    }
}

fn open_file_documents(
    values: Vec<UriRef<String>>,
    content_field: &str,
    generate_id: bool,
) -> Result<Input> {
    let (paths, origins) = resolve_file_document_paths(values)?;
    open_file_documents_from_paths(paths, origins, content_field, generate_id)
}

fn open_file_documents_from_paths(
    paths: Vec<PathBuf>,
    origins: Vec<OriginMetadata>,
    content_field: &str,
    generate_id: bool,
) -> Result<Input> {
    let source = format!("{} file document(s)", paths.len());
    Ok(Input::FileDocuments {
        source,
        paths,
        origins,
        path_index: 0,
        documents: Vec::new(),
        document_index: 0,
        content_field: content_field.to_string(),
        generate_id,
        bundle_id: if generate_id {
            bundle_identifier()?
        } else {
            String::new()
        },
    })
}

fn read_file_document_line(input: &mut Input) -> Result<InputDocument> {
    let Input::FileDocuments {
        paths,
        origins,
        path_index,
        documents,
        document_index,
        content_field,
        generate_id,
        bundle_id,
        ..
    } = input
    else {
        return Err(eyre!("Input is not a file document import"));
    };

    loop {
        if let Some(document) = documents.get(*document_index) {
            let document_index_value = *document_index;
            *document_index += 1;
            let raw = RawValue::from_string(document.get().to_string())?;
            let has_explicit_id = serde_json::from_str::<Value>(raw.get())
                .ok()
                .and_then(|value| value.as_object().map(|object| object.contains_key("_id")))
                .unwrap_or(false);
            let generated_id = if *generate_id && !has_explicit_id {
                let path = paths
                    .get(path_index.saturating_sub(1))
                    .ok_or_else(|| eyre!("File document path cursor is invalid"))?;
                Some(file_document_id(
                    bundle_id,
                    path,
                    DocumentDiscriminator::Record(document_index_value),
                )?)
            } else {
                None
            };
            return Ok(InputDocument { raw, generated_id });
        }

        let Some(path) = paths.get(*path_index) else {
            return Err(eyre!("No file document"));
        };
        let origin = origins.get(*path_index);
        *path_index += 1;
        *documents = read_file_documents(path, content_field, origin)?;
        *document_index = 0;
    }
}

fn resolve_file_document_paths(
    values: Vec<UriRef<String>>,
) -> Result<(Vec<PathBuf>, Vec<OriginMetadata>)> {
    resolve_file_document_paths_with_options(values, DiscoveryOptions::default())
}

fn resolve_file_document_paths_with_options(
    values: Vec<UriRef<String>>,
    discovery_options: DiscoveryOptions,
) -> Result<(Vec<PathBuf>, Vec<OriginMetadata>)> {
    let discovery = values.len() > 1
        || values
            .iter()
            .any(|value| has_glob_metachar(value.path().as_str()));
    let mut paths = BTreeMap::new();
    for value in values {
        let value_path = value.path().as_str().to_string();
        if has_glob_metachar(&value_path) {
            let mut matched_regular_file = false;
            for entry in glob(&value_path)
                .map_err(|err| eyre!("Invalid glob pattern {value_path}: {err}"))?
            {
                let path =
                    entry.map_err(|err| eyre!("Error expanding glob {value_path}: {err}"))?;
                if should_include_discovery_path(&path, discovery, discovery_options)?
                    && path.is_file()
                {
                    matched_regular_file = true;
                    let path = normalize_local_path(&path)?;
                    paths
                        .entry(path.clone())
                        .or_insert_with(|| origin_from_local_path(&path));
                }
            }
            if !matched_regular_file {
                return Err(eyre!("Glob matched no regular files: {value_path}"));
            }
        } else {
            let path = PathBuf::from(&value_path);
            if !should_include_discovery_path(&path, discovery, discovery_options)? {
                continue;
            }
            if !path.exists() {
                return Err(eyre!("File input does not exist: {}", path.display()));
            }
            if !path.is_file() {
                return Err(eyre!(
                    "File input is not a regular file: {}",
                    path.display()
                ));
            }
            let path = normalize_local_path(&path)?;
            paths
                .entry(path.clone())
                .or_insert_with(|| origin_from_local_path(&path));
        }
    }
    for path in paths.keys() {
        let path_str = path.to_string_lossy();
        if is_unsupported_compressed_input(path_str.as_ref())
            || (paths.len() > 1 && is_compressed_input(path_str.as_ref()))
        {
            return Err(eyre!("Unsupported compressed input format: {path_str}"));
        }
    }
    if paths.is_empty() {
        let kind = "file inputs";
        return Err(eyre!("No regular files resolved from {kind}"));
    }
    if paths.len() > 1 {
        reject_paths_outside_working_directory(paths.keys(), discovery_options.symlinks)?;
    }
    let (paths, origins): (Vec<_>, Vec<_>) = paths.into_iter().unzip();
    Ok((paths, origins))
}

fn reject_paths_outside_working_directory<'a>(
    paths: impl IntoIterator<Item = &'a PathBuf>,
    symlink_mode: SymlinkMode,
) -> Result<()> {
    let lexical_working_dir = std::env::current_dir()?;
    let canonical_working_dir = fs::canonicalize(&lexical_working_dir)?;
    for path in paths {
        let canonical_path = fs::canonicalize(path)?;
        let lexical_path_is_inside = path.starts_with(&lexical_working_dir);
        let allowed_external_symlink = symlink_mode == SymlinkMode::Follow
            && lexical_path_is_inside
            && path_contains_symlink(path)?;
        if !canonical_path.starts_with(&canonical_working_dir) && !allowed_external_symlink {
            return Err(eyre!(
                "Multi-source file input is outside the working directory: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn should_include_discovery_path(
    path: &Path,
    discovery: bool,
    options: DiscoveryOptions,
) -> Result<bool> {
    if !discovery {
        return Ok(true);
    }
    if path_contains_hidden_component(path) {
        match options.hidden {
            HiddenMode::Include => {}
            HiddenMode::Skip => return Ok(false),
            HiddenMode::Fail => {
                return Err(eyre!(
                    "Hidden path encountered in multi-source input: {}",
                    path.display()
                ));
            }
        }
    }
    if path_contains_symlink(path)? {
        match options.symlinks {
            SymlinkMode::Follow => {}
            SymlinkMode::Skip => return Ok(false),
            SymlinkMode::Fail => {
                return Err(eyre!(
                    "Symlink path encountered in multi-source input: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(true)
}

fn path_contains_hidden_component(path: &Path) -> bool {
    let path = if path.is_absolute() {
        std::env::current_dir()
            .ok()
            .map(|working_dir| relative_path_from_working_dir(path, &working_dir))
            .unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    };
    path.components().any(|component| {
        matches!(component, Component::Normal(value) if value.to_string_lossy().starts_with('.'))
    })
}

fn path_contains_symlink(path: &Path) -> Result<bool> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => current.push(".."),
            Component::Normal(value) => current.push(value),
            Component::RootDir | Component::Prefix(_) => current.push(component.as_os_str()),
        }
        if current.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
}

fn has_glob_metachar(value: &str) -> bool {
    value.bytes().any(|byte| matches!(byte, b'*' | b'?' | b'['))
}

fn should_use_file_document(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("md" | "markdown" | "txt" | "text" | "log" | "yml" | "yaml" | "jsonl")
    )
}

fn read_file_documents(
    path: &Path,
    content_field: &str,
    origin: Option<&OriginMetadata>,
) -> Result<Vec<Box<RawValue>>> {
    match extension(path).as_deref() {
        Some("ndjson" | "jsonl") => read_ndjson_file_documents(path, origin),
        Some("json") => read_json_file_document(path, origin),
        Some("toon") => read_toon_file_documents(path, origin),
        Some("yml" | "yaml") => read_yaml_file_document(path, content_field, origin),
        Some("md" | "markdown") => read_markdown_file_document(path, content_field, origin),
        _ if anydoc::Format::from_path(path)
            .is_some_and(|format| format != anydoc::Format::Csv) =>
        {
            read_anydoc_file_document(path, content_field, origin)
        }
        _ => read_text_file_document(path, content_field, origin),
    }
}

fn read_text_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|err| eyre!("{}: {err}", path.display()))?;
    String::from_utf8(bytes).map_err(|_| eyre!("{}: file is not valid UTF-8 text", path.display()))
}

fn read_text_file_document(
    path: &Path,
    content_field: &str,
    origin: Option<&OriginMetadata>,
) -> Result<Vec<Box<RawValue>>> {
    let text = read_text_file(path)?;
    let mut document = base_file_document(origin);
    document.insert(
        "content".to_string(),
        Value::Object(Map::from_iter([(
            content_field.to_string(),
            Value::String(text),
        )])),
    );
    raw_documents(vec![document])
}

fn read_markdown_file_document(
    path: &Path,
    content_field: &str,
    origin: Option<&OriginMetadata>,
) -> Result<Vec<Box<RawValue>>> {
    let text = read_text_file(path)?;
    read_markdown_text_document(path, &text, content_field, origin)
}

#[derive(Debug)]
struct AnyDocConversionError {
    path: PathBuf,
    source: anydoc::ConvertError,
}

impl std::fmt::Display for AnyDocConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.source)
    }
}

impl std::error::Error for AnyDocConversionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

fn read_anydoc_file_document(
    path: &Path,
    content_field: &str,
    origin: Option<&OriginMetadata>,
) -> Result<Vec<Box<RawValue>>> {
    let markdown = anydoc::to_markdown(path).map_err(|source| {
        Report::new(AnyDocConversionError {
            path: path.to_path_buf(),
            source,
        })
    })?;
    read_markdown_text_document(path, &markdown, content_field, origin)
}

fn read_markdown_text_document(
    path: &Path,
    text: &str,
    content_field: &str,
    origin: Option<&OriginMetadata>,
) -> Result<Vec<Box<RawValue>>> {
    let (frontmatter, body) = split_markdown_frontmatter(text);
    let mut content = Map::new();
    if let Some(frontmatter) = frontmatter {
        let (frontmatter_content, duplicate_keys) = yaml_frontmatter_to_json_map(frontmatter)
            .map_err(|err| eyre!("{}: invalid frontmatter: {err}", path.display()))?;
        for key in duplicate_keys {
            log::warn!(
                "{}: duplicate frontmatter key {key:?}; using the last value",
                path.display()
            );
        }
        content = frontmatter_content;
        if content.contains_key(content_field) {
            return Err(eyre!(
                "{}: frontmatter field conflicts with content field '{content_field}'",
                path.display()
            ));
        }
    }
    content.insert(content_field.to_string(), Value::String(body.to_string()));
    let mut document = base_file_document(origin);
    document.insert("content".to_string(), Value::Object(content));
    raw_documents(vec![document])
}

fn split_markdown_frontmatter(text: &str) -> (Option<&str>, &str) {
    let Some(after_open) = text.strip_prefix("---") else {
        return (None, text);
    };
    let after_open = after_open
        .strip_prefix("\r\n")
        .or_else(|| after_open.strip_prefix('\n'));
    let Some(after_open) = after_open else {
        return (None, text);
    };
    for delimiter in ["\n---\r\n", "\n---\n"] {
        if let Some(index) = after_open.find(delimiter) {
            let frontmatter = &after_open[..index];
            let body = &after_open[index + delimiter.len()..];
            return (Some(frontmatter), body);
        }
    }
    if let Some(frontmatter) = after_open.strip_suffix("\n---") {
        return (Some(frontmatter), "");
    }
    (None, text)
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

fn read_yaml_file_document(
    path: &Path,
    content_field: &str,
    origin: Option<&OriginMetadata>,
) -> Result<Vec<Box<RawValue>>> {
    let text = read_text_file(path)?;
    let content = yaml_mapping_to_json_map(&text)
        .map_err(|err| eyre!("{}: invalid YAML document shape: {err}", path.display()))?;
    if content.contains_key(content_field) {
        return Err(eyre!(
            "{}: YAML field conflicts with content field '{content_field}'",
            path.display()
        ));
    }
    let mut document = base_file_document(origin);
    document.insert("content".to_string(), Value::Object(content));
    raw_documents(vec![document])
}

#[derive(Debug)]
struct LenientYamlValue {
    value: serde_yaml::Value,
    duplicate_keys: Vec<String>,
}

impl<'de> serde::Deserialize<'de> for LenientYamlValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct LenientYamlValueVisitor;

        impl<'de> Visitor<'de> for LenientYamlValueVisitor {
            type Value = LenientYamlValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("any YAML value")
            }

            fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LenientYamlValue {
                    value: serde_yaml::Value::Bool(value),
                    duplicate_keys: Vec::new(),
                })
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LenientYamlValue {
                    value: serde_yaml::Value::Number(value.into()),
                    duplicate_keys: Vec::new(),
                })
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LenientYamlValue {
                    value: serde_yaml::Value::Number(value.into()),
                    duplicate_keys: Vec::new(),
                })
            }

            fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LenientYamlValue {
                    value: serde_yaml::Value::Number(value.into()),
                    duplicate_keys: Vec::new(),
                })
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_string(value.to_owned())
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LenientYamlValue {
                    value: serde_yaml::Value::String(value),
                    duplicate_keys: Vec::new(),
                })
            }

            fn visit_unit<E>(self) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(LenientYamlValue {
                    value: serde_yaml::Value::Null,
                    duplicate_keys: Vec::new(),
                })
            }

            fn visit_none<E>(self) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_unit()
            }

            fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                serde::Deserialize::deserialize(deserializer)
            }

            fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                let mut duplicate_keys = Vec::new();
                while let Some(value) = sequence.next_element::<LenientYamlValue>()? {
                    duplicate_keys.extend(value.duplicate_keys);
                    values.push(value.value);
                }
                Ok(LenientYamlValue {
                    value: serde_yaml::Value::Sequence(values),
                    duplicate_keys,
                })
            }

            fn visit_map<A>(self, mut mapping: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = serde_yaml::Mapping::new();
                let mut duplicate_keys = Vec::new();
                while let Some(key) = mapping.next_key::<serde_yaml::Value>()? {
                    let value = mapping.next_value::<LenientYamlValue>()?;
                    if values.contains_key(&key) {
                        duplicate_keys.push(yaml_key_label(&key));
                    }
                    duplicate_keys.extend(value.duplicate_keys);
                    values.insert(key, value.value);
                }
                Ok(LenientYamlValue {
                    value: serde_yaml::Value::Mapping(values),
                    duplicate_keys,
                })
            }

            fn visit_enum<A>(self, data: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: EnumAccess<'de>,
            {
                let (tag, contents) = data.variant::<String>()?;
                let value = contents.newtype_variant::<LenientYamlValue>()?;
                Ok(LenientYamlValue {
                    value: serde_yaml::Value::Tagged(Box::new(serde_yaml::value::TaggedValue {
                        tag: serde_yaml::value::Tag::new(tag),
                        value: value.value,
                    })),
                    duplicate_keys: value.duplicate_keys,
                })
            }
        }

        deserializer.deserialize_any(LenientYamlValueVisitor)
    }
}

fn yaml_key_label(key: &serde_yaml::Value) -> String {
    match key {
        serde_yaml::Value::String(key) => key.clone(),
        key => format!("{key:?}"),
    }
}

fn yaml_frontmatter_to_json_map(text: &str) -> Result<(Map<String, Value>, Vec<String>)> {
    let deserializer = serde_yaml::Deserializer::from_str(text);
    let yaml = <LenientYamlValue as serde::Deserialize>::deserialize(deserializer)?;
    let Value::Object(map) = serde_json::to_value(yaml.value)? else {
        return Err(eyre!("root must be a mapping"));
    };
    Ok((map, yaml.duplicate_keys))
}

fn yaml_mapping_to_json_map(text: &str) -> Result<Map<String, Value>> {
    let yaml: serde_yaml::Value = serde_yaml::from_str(text)?;
    let Value::Object(map) = serde_json::to_value(yaml)? else {
        return Err(eyre!("root must be a mapping"));
    };
    Ok(map)
}

fn read_json_file_document(
    path: &Path,
    origin: Option<&OriginMetadata>,
) -> Result<Vec<Box<RawValue>>> {
    let text = read_text_file(path)?;
    let mut document = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => map,
        Ok(Value::Array(_)) => {
            return Err(eyre!(
                "{}: .json inputs must contain one JSON object, not an array",
                path.display()
            ));
        }
        Ok(_) | Err(_) => {
            return Err(eyre!(
                "{}: .json inputs must contain one JSON object",
                path.display()
            ));
        }
    };
    add_origin_metadata(&mut document, origin);
    raw_documents(vec![document])
}

fn read_ndjson_file_documents(
    path: &Path,
    origin: Option<&OriginMetadata>,
) -> Result<Vec<Box<RawValue>>> {
    let text = read_text_file(path)?;
    let mut docs = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|err| eyre!("{}:{}: invalid JSON line: {err}", path.display(), index + 1))?;
        let Value::Object(mut document) = value else {
            return Err(eyre!(
                "{}:{}: JSON line must be an object",
                path.display(),
                index + 1
            ));
        };
        add_origin_metadata(&mut document, origin);
        docs.push(RawValue::from_string(Value::Object(document).to_string())?);
    }
    Ok(docs)
}

fn read_toon_file_documents(
    path: &Path,
    origin: Option<&OriginMetadata>,
) -> Result<Vec<Box<RawValue>>> {
    let file = File::open(path).map_err(|err| eyre!("{}: {err}", path.display()))?;
    let mut reader = BufReader::new(Box::new(file) as Box<dyn Read + Send>);
    let mut pending = String::new();
    let mut document_index = 0;
    let mut buffered_rows = Vec::new();
    let mut eof = false;
    let mut docs = Vec::new();
    let source = path.display().to_string();

    loop {
        match read_toon_document(
            &source,
            &mut reader,
            &mut pending,
            &mut document_index,
            &mut buffered_rows,
            &mut eof,
        ) {
            Ok(mut raw) => {
                if origin.is_some() {
                    let mut document: Map<String, Value> = serde_json::from_str(raw.get())?;
                    add_origin_metadata(&mut document, origin);
                    raw = RawValue::from_string(Value::Object(document).to_string())?;
                }
                docs.push(raw);
            }
            Err(err) if is_end_of_input(&err) => return Ok(docs),
            Err(err) => return Err(err),
        }
    }
}

fn decode_toon_documents(
    source: &str,
    document_index: usize,
    input: &str,
    buffered_rows: &mut Vec<Value>,
) -> Result<Box<RawValue>> {
    let value: Value = toon_format::decode_default(input).map_err(|err| {
        eyre!("{source}: document {document_index}: invalid Toon document: {err}")
    })?;
    toon_value_to_first_document(source, document_index, value, buffered_rows)
}

fn toon_value_to_first_document(
    source: &str,
    document_index: usize,
    value: Value,
    buffered_rows: &mut Vec<Value>,
) -> Result<Box<RawValue>> {
    let Value::Object(document) = value else {
        return Err(eyre!(
            "{source}: document {document_index}: Toon document must be an object"
        ));
    };

    if document.len() == 1 {
        let (key, value) = document.into_iter().next().unwrap();
        if let Value::Array(mut rows) = value {
            rows.reverse();
            let Some(first) = rows.pop() else {
                return Err(eyre!(
                    "{source}: document {document_index}: Toon document produced no rows"
                ));
            };
            buffered_rows.extend(rows);
            return toon_row_value_to_raw(source, document_index, first);
        }

        let document = Map::from_iter([(key, value)]);
        return RawValue::from_string(Value::Object(document).to_string()).map_err(Into::into);
    }

    RawValue::from_string(Value::Object(document).to_string()).map_err(Into::into)
}

fn toon_row_value_to_raw(source: &str, document_index: usize, row: Value) -> Result<Box<RawValue>> {
    let Value::Object(row) = row else {
        return Err(eyre!(
            "{source}: document {document_index}: Toon array row must be an object"
        ));
    };
    RawValue::from_string(Value::Object(row).to_string()).map_err(Into::into)
}

fn base_file_document(origin: Option<&OriginMetadata>) -> Map<String, Value> {
    let mut document = Map::new();
    add_origin_metadata(&mut document, origin);
    document
}

fn add_origin_metadata(document: &mut Map<String, Value>, origin: Option<&OriginMetadata>) {
    if let Some(origin) = origin {
        document.insert("origin".to_string(), origin.clone().into_value());
    }
}

fn origin_from_local_path(path: &Path) -> OriginMetadata {
    let path = working_relative_path(path);
    let filename = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string();
    let directory = path.parent().unwrap_or_else(|| Path::new(""));
    OriginMetadata {
        scheme: "file".to_string(),
        authority: None,
        path: origin_directory(directory),
        query: None,
        fragment: None,
        filename,
    }
}

fn normalize_local_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized.file_name().is_some() {
                    normalized.pop();
                }
            }
            Component::Normal(value) => normalized.push(value),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

fn working_relative_path(path: &Path) -> PathBuf {
    let Some(working_dir) = std::env::current_dir().ok() else {
        return path.to_path_buf();
    };
    relative_path_from_working_dir(path, &working_dir)
}

fn relative_path_from_working_dir(path: &Path, working_dir: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_dir.join(path)
    };
    let working_components: Vec<_> = working_dir.components().collect();
    let path_components: Vec<_> = absolute.components().collect();
    let common = working_components
        .iter()
        .zip(&path_components)
        .take_while(|(working, path)| working == path)
        .count();

    let mut relative = PathBuf::new();
    for _ in common..working_components.len() {
        relative.push("..");
    }
    for component in &path_components[common..] {
        relative.push(component.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    relative
}

fn bundle_identifier() -> Result<String> {
    let working_dir = std::env::current_dir()?;
    let mut candidate = working_dir.as_path();
    while let Some(parent) = candidate.parent() {
        if candidate.join(".git").exists() {
            return candidate
                .file_name()
                .and_then(OsStr::to_str)
                .map(str::to_string)
                .ok_or_else(|| eyre!("Tracked repository has no directory name"));
        }
        candidate = parent;
    }

    working_dir
        .parent()
        .and_then(Path::file_name)
        .and_then(OsStr::to_str)
        .map(str::to_string)
        .or_else(|| {
            working_dir
                .file_name()
                .and_then(OsStr::to_str)
                .map(str::to_string)
        })
        .ok_or_else(|| eyre!("Working path has no directory name"))
}

fn file_document_id(
    bundle_id: &str,
    path: &Path,
    discriminator: DocumentDiscriminator,
) -> Result<String> {
    file_document_id_for_relative_path(bundle_id, &working_relative_path(path), discriminator)
}

fn file_document_id_for_relative_path(
    bundle_id: &str,
    relative_path: &Path,
    discriminator: DocumentDiscriminator,
) -> Result<String> {
    let relative_path = relative_path
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    let relative_path = relative_path.strip_prefix("./").unwrap_or(&relative_path);
    let key = format!("{bundle_id}\0{relative_path}\0{}", discriminator.encode());
    let digest = Sha256::digest(key.as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(&digest[..16]))
}

fn origin_from_uri(uri: &UriRef<String>) -> OriginMetadata {
    let path = uri.path().as_str();
    let path_ref = Path::new(path);
    let is_directory = path.is_empty() || path.ends_with('/');
    OriginMetadata {
        scheme: uri
            .scheme()
            .map(|scheme| scheme.as_str().to_string())
            .unwrap_or_else(|| "file".to_string()),
        authority: uri
            .authority()
            .map(|authority| authority.as_str().to_string()),
        path: if is_directory {
            if path.is_empty() {
                "./".to_string()
            } else {
                path.to_string()
            }
        } else {
            origin_directory(path_ref.parent().unwrap_or_else(|| Path::new("")))
        },
        query: uri.query().map(|query| query.as_str().to_string()),
        fragment: uri.fragment().map(|fragment| fragment.as_str().to_string()),
        filename: if is_directory {
            String::new()
        } else {
            path_ref
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string()
        },
    }
}

impl OriginMetadata {
    fn into_value(self) -> Value {
        let mut object = Map::from_iter([
            ("scheme".to_string(), Value::String(self.scheme)),
            ("path".to_string(), Value::String(self.path)),
            ("filename".to_string(), Value::String(self.filename)),
        ]);
        if let Some(authority) = self.authority {
            object.insert("authority".to_string(), Value::String(authority));
        }
        if let Some(query) = self.query {
            object.insert("query".to_string(), Value::String(query));
        }
        if let Some(fragment) = self.fragment {
            object.insert("fragment".to_string(), Value::String(fragment));
        }
        Value::Object(object)
    }
}

fn origin_directory(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        "./".to_string()
    } else {
        path.to_string_lossy().into_owned()
    }
}

fn add_origin_to_raw(raw: Box<RawValue>, origin: Option<&OriginMetadata>) -> Result<Box<RawValue>> {
    let Some(origin) = origin else {
        return Ok(raw);
    };
    let Value::Object(mut document) = serde_json::from_str(raw.get())? else {
        return Err(eyre!("Input document must be a JSON object"));
    };
    document.insert("origin".to_string(), origin.clone().into_value());
    RawValue::from_string(Value::Object(document).to_string()).map_err(Into::into)
}

fn raw_documents(documents: Vec<Map<String, Value>>) -> Result<Vec<Box<RawValue>>> {
    documents
        .into_iter()
        .map(|document| {
            RawValue::from_string(Value::Object(document).to_string()).map_err(Into::into)
        })
        .collect()
}

fn fetch_remote_input(uri: UriRef<String>) -> Result<Input> {
    let client = Client::builder()
        .connect_timeout(REMOTE_CONNECT_TIMEOUT)
        .timeout(REMOTE_REQUEST_TIMEOUT)
        .build()?;
    fetch_remote_input_with_client(uri, &client)
}

fn fetch_remote_split_input(uri: UriRef<String>, split: SplitPath) -> Result<Input> {
    let client = Client::builder()
        .connect_timeout(REMOTE_CONNECT_TIMEOUT)
        .timeout(REMOTE_REQUEST_TIMEOUT)
        .build()?;
    fetch_remote_input_with_client_and_split(uri, &client, Some(split))
}

fn fetch_remote_input_with_client(uri: UriRef<String>, client: &Client) -> Result<Input> {
    fetch_remote_input_with_client_and_split(uri, client, None)
}

fn fetch_remote_input_with_client_and_split(
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
    let suffix = match kind {
        InputKind::Csv => ".csv",
        InputKind::Ndjson => ".ndjson",
        InputKind::Json => ".json",
        InputKind::Toon => ".toon",
        InputKind::FileDocument => return Err(eyre!("Unsupported remote input format")),
    };

    let mut temp_file = Builder::new().suffix(suffix).tempfile()?;
    std::io::copy(&mut response, temp_file.as_file_mut())?;
    temp_file.as_file_mut().flush()?;

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
            origin,
            None,
            Some(temp_file),
        );
    }

    match kind {
        InputKind::Csv => Ok(Input::FileCsv {
            source,
            reader: Box::new(
                csv::ReaderBuilder::new()
                    .has_headers(true)
                    .from_reader(Box::new(reader_file) as Box<dyn Read + Send>),
            ),
            origin: origin.clone(),
            file_identity: None,
            _temp_file: Some(temp_file),
        }),
        InputKind::Ndjson | InputKind::Json => Ok(Input::FileJson {
            source,
            reader: Box::new(BufReader::new(Box::new(reader_file) as Box<dyn Read + Send>)),
            first_record: true,
            origin: origin.clone(),
            file_identity: None,
            _temp_file: Some(temp_file),
        }),
        InputKind::Toon => Ok(Input::FileToon {
            source,
            reader: Box::new(BufReader::new(Box::new(reader_file) as Box<dyn Read + Send>)),
            pending: String::new(),
            document_index: 0,
            buffered_rows: Vec::new(),
            eof: false,
            origin,
            file_identity: None,
            _temp_file: Some(temp_file),
        }),
        InputKind::FileDocument => Err(eyre!("Unsupported remote input format")),
    }
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

    if content_type.contains("text/csv") || content_type.contains("application/csv") {
        return Ok(InputKind::Csv);
    }
    if content_type.contains("application/x-ndjson") || content_type.contains("application/ndjson")
    {
        return Ok(InputKind::Ndjson);
    }
    if content_type.contains("application/json") || content_type.ends_with("+json") {
        return Ok(InputKind::Json);
    }
    if content_type.contains("application/toon")
        || content_type.contains("application/x-toon")
        || content_type.contains("text/toon")
    {
        return Ok(InputKind::Toon);
    }

    Err(eyre!("Unsupported remote input format"))
}

fn local_input_kind(path: &Path) -> Result<InputKind> {
    input_kind_from_path(path.to_string_lossy().as_ref())
        .ok_or_else(|| eyre!("Unsupported file extension"))
}

fn input_kind_from_path(path: &str) -> Option<InputKind> {
    if has_path_suffix(path, ".csv.gz") {
        return Some(InputKind::Csv);
    }
    if has_path_suffix(path, ".ndjson.gz") {
        return Some(InputKind::Ndjson);
    }

    let extension = PathBuf::from(path)
        .extension()
        .and_then(OsStr::to_str)?
        .to_ascii_lowercase();
    match extension.as_str() {
        "csv" => Some(InputKind::Csv),
        "ndjson" => Some(InputKind::Ndjson),
        "json" => Some(InputKind::Json),
        "toon" => Some(InputKind::Toon),
        "md" | "markdown" | "txt" | "text" | "log" | "yml" | "yaml" | "jsonl" => {
            Some(InputKind::FileDocument)
        }
        _ => None,
    }
}

fn local_file_reader(file: File, path: &Path) -> Box<dyn Read + Send> {
    if has_path_suffix(path.to_string_lossy().as_ref(), ".gz") {
        return Box::new(GzDecoder::new(file));
    }
    Box::new(file)
}

fn has_path_suffix(path: &str, suffix: &str) -> bool {
    path.len() >= suffix.len()
        && path
            .get(path.len() - suffix.len()..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
}

fn is_compressed_input(path: &str) -> bool {
    has_path_suffix(path, ".gz")
}

fn is_unsupported_compressed_input(path: &str) -> bool {
    is_compressed_input(path)
        && !has_path_suffix(path, ".csv.gz")
        && !has_path_suffix(path, ".ndjson.gz")
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
}

fn validate_ndjson_file(file: &mut File) -> Result<()> {
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

fn ensure_json_opening(input: &str, error_message: &str) -> Result<()> {
    match input.bytes().find(|byte| !byte.is_ascii_whitespace()) {
        Some(b'{') => Ok(()),
        _ => Err(eyre!(error_message.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DiscoveryOptions, DocumentDiscriminator, HiddenMode, Input, InputDocument, InputKind,
        JSON_LINE_OPENING_ERROR, REMOTE_NDJSON_ERROR, SymlinkMode, bundle_identifier,
        fetch_remote_input_with_client, fetch_remote_input_with_client_and_split, file_document_id,
        file_document_id_for_relative_path, input_kind_from_path, local_input_kind,
        normalize_local_path, open_file_documents, open_input_values,
        open_input_values_with_generate_id, open_input_values_with_generate_id_and_options,
        open_split_inputs, origin_from_local_path, origin_from_uri, relative_path_from_working_dir,
        validate_content_field, validate_ndjson_file,
    };
    use crate::json_split::SplitPath;
    use base64::Engine as _;
    use flate2::{Compression, write::GzEncoder};
    use fluent_uri::UriRef;
    use reqwest::blocking::Client;
    use rustls::{
        ServerConfig, ServerConnection, StreamOwned,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::{
        fs,
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        path::{Path, PathBuf},
        sync::{Arc, mpsc},
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };
    use tempfile::NamedTempFile;

    fn uri(path: &PathBuf) -> UriRef<String> {
        UriRef::parse(path.to_string_lossy().into_owned()).unwrap()
    }

    fn collect_values(mut input: Input) -> Vec<serde_json::Value> {
        let mut values = Vec::new();
        let mut line = String::new();
        while let Ok(value) = input.read_line(&mut line) {
            values.push(serde_json::from_str(value.get()).unwrap());
            line.clear();
        }
        values
    }

    fn collect_documents(mut input: Input) -> Vec<InputDocument> {
        let mut documents = Vec::new();
        let mut line = String::new();
        while let Ok(value) = input.read_line(&mut line) {
            documents.push(value);
            line.clear();
        }
        documents
    }

    fn input_err(result: eyre::Result<Input>) -> String {
        match result {
            Ok(_) => panic!("expected input construction to fail"),
            Err(err) => err.to_string(),
        }
    }

    fn read_err(result: eyre::Result<Input>) -> String {
        let mut input = result.unwrap();
        let mut line = String::new();
        input.read_line(&mut line).unwrap_err().to_string()
    }

    fn temp_path(suffix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("espipe-input-{nanos}.{suffix}"))
    }

    fn workspace_tempdir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("espipe-test-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    fn write_base64_fixture(source: &str, path: &Path) {
        let encoded = fs::read_to_string(fixture_path(source)).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn write_gzip(path: &PathBuf, contents: &str) {
        let file = fs::File::create(path).unwrap();
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(contents.as_bytes()).unwrap();
        encoder.finish().unwrap();
    }

    #[test]
    fn input_kind_detects_supported_compressed_suffixes() {
        assert_eq!(
            input_kind_from_path("/tmp/events.csv.gz"),
            Some(InputKind::Csv)
        );
        assert_eq!(
            input_kind_from_path("/tmp/events.ndjson.gz"),
            Some(InputKind::Ndjson)
        );
        assert_eq!(input_kind_from_path("/tmp/events.json.gz"), None);
        assert_eq!(
            input_kind_from_path("/tmp/events.csv"),
            Some(InputKind::Csv)
        );
        assert_eq!(
            input_kind_from_path("/tmp/events.ndjson"),
            Some(InputKind::Ndjson)
        );
        assert_eq!(
            input_kind_from_path("/tmp/events.json"),
            Some(InputKind::Json)
        );
        assert_eq!(
            input_kind_from_path("/tmp/events.toon"),
            Some(InputKind::Toon)
        );
        assert_eq!(input_kind_from_path("/tmp/events.toon.gz"), None);
    }

    #[test]
    fn read_line_adds_origin_to_ndjson() {
        let path = temp_path("ndjson");
        fs::write(&path, "{\"a\":1}\n").unwrap();
        let mut input =
            Input::try_from(UriRef::parse(path.to_string_lossy().into_owned()).unwrap()).unwrap();

        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
        assert_eq!(actual["a"], 1);
        assert_eq!(actual["origin"]["scheme"], "file");
        assert_eq!(
            actual["origin"]["filename"],
            path.file_name().unwrap().to_str().unwrap()
        );

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_line_converts_csv_to_raw_json() {
        let path = temp_path("csv");
        fs::write(&path, "name,count\nalpha,2\n").unwrap();
        let mut input =
            Input::try_from(UriRef::parse(path.to_string_lossy().into_owned()).unwrap()).unwrap();

        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
        assert_eq!(actual["name"], "alpha");
        assert_eq!(actual["count"], "2");
        assert_eq!(actual["origin"]["scheme"], "file");
        assert_eq!(
            actual["origin"]["filename"],
            path.file_name().unwrap().to_str().unwrap()
        );

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_line_converts_gzip_csv_to_raw_json() {
        let path = temp_path("csv.gz");
        write_gzip(&path, "name,count\nalpha,2\n");
        let mut input =
            Input::try_from(UriRef::parse(path.to_string_lossy().into_owned()).unwrap()).unwrap();

        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
        assert_eq!(actual["name"], "alpha");
        assert_eq!(actual["count"], "2");
        assert_eq!(actual["origin"]["scheme"], "file");
        assert_eq!(
            actual["origin"]["filename"],
            path.file_name().unwrap().to_str().unwrap()
        );

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_line_adds_origin_to_gzip_ndjson() {
        let path = temp_path("ndjson.gz");
        write_gzip(&path, "{\"a\":1}\n");
        let mut input =
            Input::try_from(UriRef::parse(path.to_string_lossy().into_owned()).unwrap()).unwrap();

        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
        assert_eq!(actual["a"], 1);
        assert_eq!(actual["origin"]["scheme"], "file");
        assert_eq!(
            actual["origin"]["filename"],
            path.file_name().unwrap().to_str().unwrap()
        );

        fs::remove_file(path).unwrap();
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
    fn direct_markdown_file_imports_default_content_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "# Title\nBody\n").unwrap();

        let values = collect_values(Input::try_from(uri(&path)).unwrap());

        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["content"]["body"], "# Title\nBody\n");
        assert_eq!(values[0]["origin"]["scheme"], "file");
        assert_eq!(values[0]["origin"]["filename"], "note.md");
    }

    #[test]
    fn generated_file_id_is_stable_when_content_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "first").unwrap();

        let first = collect_documents(open_file_documents(vec![uri(&path)], "body", true).unwrap());
        let first_id = first[0].generated_id.clone().unwrap();
        assert_eq!(first_id.len(), 22);

        fs::write(&path, "second").unwrap();
        let second =
            collect_documents(open_file_documents(vec![uri(&path)], "body", true).unwrap());
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
    fn multi_source_inputs_can_fail_on_symlinks() {
        let dir = workspace_tempdir();
        let external = tempfile::tempdir().unwrap();
        let external_file = external.path().join("external.txt");
        fs::write(&external_file, "external").unwrap();
        let link = dir.path().join("link.txt");
        symlink(&external_file, &link).unwrap();
        let local = dir.path().join("local.txt");
        fs::write(&local, "local").unwrap();

        let err = input_err(open_input_values_with_generate_id_and_options(
            vec![uri(&link), uri(&local)],
            "body",
            None,
            DiscoveryOptions {
                symlinks: SymlinkMode::Fail,
                hidden: HiddenMode::Skip,
            },
        ));
        assert!(err.contains("Symlink path encountered"));
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
    fn multi_source_inputs_reject_external_symlink_paths_even_when_following() {
        let dir = workspace_tempdir();
        let external = tempfile::Builder::new()
            .prefix("espipe-external-")
            .tempdir_in(std::env::temp_dir())
            .unwrap();
        let external_file = external.path().join("external.txt");
        fs::write(&external_file, "external").unwrap();
        let link = external.path().join("link.txt");
        symlink(&external_file, &link).unwrap();
        let local = dir.path().join("local.txt");
        fs::write(&local, "local").unwrap();
        let result = open_input_values_with_generate_id_and_options(
            vec![uri(&link), uri(&local)],
            "body",
            None,
            DiscoveryOptions {
                symlinks: SymlinkMode::Follow,
                hidden: HiddenMode::Skip,
            },
        );
        let err = input_err(result);
        assert!(err.contains("outside the working directory"));
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
    fn single_file_glob_uses_streaming_parser_for_structured_input() {
        let dir = workspace_tempdir();
        let input = dir.path().join("records.ndjson");
        fs::write(&input, "{\"message\":\"hello\"}\n").unwrap();
        let pattern = dir.path().join("*.ndjson");

        let input = open_input_values(vec![uri(&pattern)], "body").unwrap();

        assert!(matches!(input, Input::FileJson { .. }));
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

        let first = file_document_id_for_relative_path(
            "bundle",
            &first_path,
            DocumentDiscriminator::Record(0),
        )
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
        let explicit = collect_documents(
            open_file_documents(vec![uri(&explicit_path)], "body", true).unwrap(),
        );
        assert!(explicit[0].generated_id.is_none());

        let path = dir.path().join("note.md");
        fs::write(&path, "hello").unwrap();
        let disabled =
            collect_documents(open_file_documents(vec![uri(&path)], "body", false).unwrap());
        assert!(disabled[0].generated_id.is_none());
    }

    #[test]
    fn generated_file_ids_include_document_index_for_multi_document_files() {
        let path = fixture_path("multi.toon");
        let documents =
            collect_documents(open_file_documents(vec![uri(&path)], "body", true).unwrap());

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
        let root = origin_from_uri(&UriRef::parse("https://example.com/".to_string()).unwrap())
            .into_value();
        assert_eq!(root["path"], "/");
        assert_eq!(root["filename"], "");

        let directory =
            origin_from_uri(&UriRef::parse("https://example.com/docs/".to_string()).unwrap())
                .into_value();
        assert_eq!(directory["path"], "/docs/");
        assert_eq!(directory["filename"], "");
    }

    #[test]
    fn anydoc_converts_pdf_to_default_markdown_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.pdf");
        write_base64_fixture("anydoc/sample.pdf.base64", &path);
        let values = collect_values(Input::try_from(uri(&path)).unwrap());

        assert_eq!(values.len(), 1);
        assert!(
            values[0]["content"]["body"]
                .as_str()
                .unwrap()
                .contains("Hello PDF")
        );
        assert_eq!(values[0]["origin"]["scheme"], "file");
        assert_eq!(values[0]["origin"]["filename"], "sample.pdf");
    }

    #[test]
    fn anydoc_converts_rtf_to_custom_content_field() {
        let path = fixture_path("anydoc/sample.rtf");
        let values = collect_values(open_input_values(vec![uri(&path)], "markdown").unwrap());

        assert!(
            values[0]["content"]["markdown"]
                .as_str()
                .unwrap()
                .contains("Hello from RTF")
        );
        assert!(values[0]["content"].get("body").is_none());
    }

    #[test]
    fn anydoc_converts_office_docx_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.docx");
        write_base64_fixture("anydoc/sample.docx.base64", &path);

        let values = collect_values(Input::try_from(uri(&path)).unwrap());

        assert_eq!(values.len(), 1);
        assert!(
            values[0]["content"]["body"]
                .as_str()
                .unwrap()
                .contains("Hello from DOCX")
        );
    }

    #[test]
    fn anydoc_mixed_file_import_sorts_paths_and_preserves_origin_metadata() {
        let dir = workspace_tempdir();
        let pdf = dir.path().join("sample.pdf");
        write_base64_fixture("anydoc/sample.pdf.base64", &pdf);
        let rtf = dir.path().join("sample.rtf");
        fs::copy(fixture_path("anydoc/sample.rtf"), &rtf).unwrap();
        let values = collect_values(open_input_values(vec![uri(&rtf), uri(&pdf)], "body").unwrap());

        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["origin"]["scheme"], "file");
        assert!(values[0]["origin"].get("authority").is_none());
        assert_eq!(values[0]["origin"]["filename"], "sample.pdf");
        assert!(
            !values[0]["origin"]["path"]
                .as_str()
                .unwrap()
                .starts_with('/')
        );
        assert!(values[0]["origin"].get("query").is_none());
        assert!(values[0]["origin"].get("fragment").is_none());
        assert_eq!(values[1]["origin"]["filename"], "sample.rtf");
        assert!(values[0]["content"]["body"].is_string());
        assert!(values[1]["content"]["body"].is_string());
    }

    #[test]
    fn anydoc_recursive_glob_imports_pdf_files() {
        let dir = workspace_tempdir();
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).unwrap();
        write_base64_fixture("anydoc/sample.pdf.base64", &nested.join("sample.pdf"));
        let pattern = dir
            .path()
            .join("**")
            .join("*.pdf")
            .to_string_lossy()
            .into_owned();

        let values = collect_values(
            open_input_values(vec![UriRef::parse(pattern).unwrap()], "body").unwrap(),
        );

        assert_eq!(values.len(), 1);
        assert!(values[0]["content"]["body"].is_string());
        assert_eq!(values[0]["origin"]["filename"], "sample.pdf");
        assert!(
            !values[0]["origin"]["path"]
                .as_str()
                .unwrap()
                .starts_with('/')
        );
    }

    #[test]
    fn anydoc_multiple_extension_globs_combine_and_sort_inputs() {
        let dir = workspace_tempdir();
        let pdf = dir.path().join("a.pdf");
        let rtf = dir.path().join("b.rtf");
        write_base64_fixture("anydoc/sample.pdf.base64", &pdf);
        fs::copy(fixture_path("anydoc/sample.rtf"), &rtf).unwrap();
        let pdf_pattern = dir.path().join("**/*.pdf").to_string_lossy().into_owned();
        let rtf_pattern = dir.path().join("**/*.rtf").to_string_lossy().into_owned();

        let values = collect_values(
            open_input_values(
                vec![
                    UriRef::parse(rtf_pattern).unwrap(),
                    UriRef::parse(pdf_pattern).unwrap(),
                ],
                "body",
            )
            .unwrap(),
        );

        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["origin"]["filename"], "a.pdf");
        assert_eq!(values[1]["origin"]["filename"], "b.rtf");
    }

    #[test]
    fn anydoc_conversion_error_includes_source_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.pdf");
        fs::write(&path, b"not a PDF").unwrap();

        let err = read_err(open_input_values(vec![uri(&path)], "body"));

        assert!(err.contains("invalid.pdf"));
        assert!(err.len() > "invalid.pdf".len());
    }

    #[test]
    fn anydoc_rejects_image_only_pdf_with_ocr_diagnostic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image-only.pdf");
        write_base64_fixture("anydoc/image-only.pdf.base64", &path);

        let err = read_err(open_input_values(vec![uri(&path)], "body"));

        assert!(err.contains("image-only.pdf"));
        assert!(err.contains("OCR is required"));
    }

    #[test]
    fn shell_expanded_files_are_sorted_deduplicated_and_include_origin_metadata() {
        let dir = workspace_tempdir();
        let b = dir.path().join("b.txt");
        let a = dir.path().join("a.txt");
        fs::write(&b, "bravo").unwrap();
        fs::write(&a, "alpha").unwrap();

        let input = open_input_values(vec![uri(&b), uri(&a), uri(&a)], "body").unwrap();
        let values = collect_values(input);

        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["content"]["body"], "alpha");
        assert_eq!(values[1]["content"]["body"], "bravo");
        assert_eq!(values[0]["origin"]["filename"], "a.txt");
        assert_eq!(values[1]["origin"]["filename"], "b.txt");
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
        assert_eq!(values[0]["content"]["body"], "child");
        assert_eq!(values[1]["content"]["body"], "root");
        assert_eq!(values[0]["origin"]["filename"], "child.md");
        assert!(
            !values[0]["origin"]["path"]
                .as_str()
                .unwrap()
                .starts_with('/')
        );
        assert_eq!(values[1]["origin"]["filename"], "root.md");
        assert!(
            !values[1]["origin"]["path"]
                .as_str()
                .unwrap()
                .starts_with('/')
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

    #[test]
    fn content_field_validation_rejects_empty_and_dotted_names() {
        assert!(validate_content_field("body").is_ok());
        assert!(validate_content_field("markdown").is_ok());
        assert!(
            validate_content_field("")
                .unwrap_err()
                .to_string()
                .contains("empty")
        );
        assert!(
            validate_content_field("page.body")
                .unwrap_err()
                .to_string()
                .contains("must not contain")
        );
    }

    #[test]
    fn custom_content_field_is_used_without_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        fs::write(&path, "hello").unwrap();

        let values = collect_values(open_input_values(vec![uri(&path)], "markdown").unwrap());

        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["content"]["markdown"], "hello");
        assert_eq!(values[0]["origin"]["scheme"], "file");
        assert_eq!(values[0]["origin"]["filename"], "note.txt");
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

        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["content"]["count"], 2);
        assert_eq!(values[0]["content"]["title"], "Hello");
        assert_eq!(values[0]["origin"]["scheme"], "file");
        assert_eq!(values[0]["origin"]["filename"], "doc.yml");

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
    fn file_document_import_reads_files_lazily() {
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
        let err = input.read_line(&mut line).unwrap_err();
        assert!(err.to_string().contains("not valid UTF-8"));
    }

    #[test]
    fn json_file_document_requires_whole_object() {
        let dir = workspace_tempdir();
        let path = dir.path().join("doc.json");
        fs::write(&path, "{\"a\":1}").unwrap();

        let values =
            collect_values(open_input_values(vec![uri(&path), uri(&path)], "body").unwrap());
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["a"], 1);
        assert_eq!(values[0]["origin"]["scheme"], "file");
        assert_eq!(values[0]["origin"]["filename"], "doc.json");

        fs::write(&path, "[1,2]").unwrap();
        let err = read_err(open_input_values(vec![uri(&path), uri(&path)], "body"));
        assert!(err.contains("must contain one JSON object"));
    }

    #[test]
    fn jsonl_streams_object_lines_and_rejects_non_objects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.jsonl");
        fs::write(&path, "{\"a\":1}\n\n{\"b\":2}\n").unwrap();

        let values = collect_values(open_input_values(vec![uri(&path)], "body").unwrap());
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["a"], 1);
        assert_eq!(values[1]["b"], 2);
        assert_eq!(values[0]["origin"]["filename"], "doc.jsonl");
        assert_eq!(values[1]["origin"]["filename"], "doc.jsonl");

        fs::write(&path, "[1,2]\n").unwrap();
        let err = read_err(open_input_values(vec![uri(&path)], "body"));
        assert!(err.contains("JSON line must be an object"));
    }

    #[test]
    fn toon_file_streams_object_documents_in_order() {
        let values = collect_values(Input::try_from(uri(&fixture_path("multi.toon"))).unwrap());

        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["id"], 1);
        assert_eq!(values[0]["name"], "Alpha");
        assert_eq!(values[1]["id"], 2);
        assert_eq!(values[1]["name"], "Bravo");
        assert_eq!(values[1]["tags"], serde_json::json!(["search", "bulk"]));
        assert_eq!(values[0]["origin"]["filename"], "multi.toon");
        assert_eq!(values[1]["origin"]["filename"], "multi.toon");
    }

    #[test]
    fn toon_root_tabular_array_emits_one_document_per_row() {
        let values =
            collect_values(Input::try_from(uri(&fixture_path("measurements.toon"))).unwrap());

        assert_eq!(values.len(), 3);
        assert_eq!(values[0]["@timestamp"], "2026-05-06T18:42:00Z");
        assert_eq!(values[0]["evaluation"], "force-merge-20260506T184200Z");
        assert_eq!(values[0]["metric"], "search_latency_p99");
        assert_eq!(values[0]["value"], 100.0);
        assert_eq!(values[1]["variation"], "candidate");
        assert_eq!(values[1]["value"], 150.0);
        assert_eq!(values[2]["metric"], "throughput");
        assert_eq!(values[2]["unit"], "docs/s");
        assert_eq!(values[2]["artifact"], "comparison.toon");
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

        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["active"], true);
        assert_eq!(values[0]["id"], 1);
        assert_eq!(values[0]["name"], "Alpha");
        assert_eq!(values[0]["origin"]["filename"], "single.toon");
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

        let values =
            collect_values(open_input_values(vec![uri(&text), uri(&toon)], "body").unwrap());

        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["content"]["body"], "alpha");
        assert_eq!(values[1]["id"], 2);
        assert_eq!(values[1]["origin"]["filename"], "b.toon");
    }

    #[test]
    fn invalid_utf8_file_document_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.txt");
        fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();

        let err = read_err(open_input_values(vec![uri(&path)], "body"));

        assert!(err.contains("not valid UTF-8"));
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
    fn json_validation_rejects_non_ndjson_payload() {
        let mut temp = NamedTempFile::new().unwrap();
        writeln!(temp, "\"hello\"").unwrap();

        let err = validate_ndjson_file(temp.as_file_mut()).unwrap_err();
        assert_eq!(err.to_string(), REMOTE_NDJSON_ERROR);
    }

    #[test]
    fn json_validation_rejects_array_payload() {
        let mut temp = NamedTempFile::new().unwrap();
        writeln!(temp, "[1,2]").unwrap();

        let err = validate_ndjson_file(temp.as_file_mut()).unwrap_err();
        assert_eq!(err.to_string(), REMOTE_NDJSON_ERROR);
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
    fn remote_http_fetch_preserves_origin_uri_components() {
        let (base_url, _requests, handle) =
            spawn_http_server("200 OK", "text/csv", "name,count\nalpha,2\n");
        let uri =
            UriRef::parse(format!("{base_url}/docs/data.csv?download=1#row").to_string()).unwrap();
        let authority = uri.authority().unwrap().as_str().to_string();
        let values = collect_values(
            fetch_remote_input_with_client(uri, &Client::builder().build().unwrap()).unwrap(),
        );

        assert_eq!(values[0]["name"], "alpha");
        assert_eq!(values[0]["origin"]["scheme"], "http");
        assert_eq!(values[0]["origin"]["authority"], authority);
        assert_eq!(values[0]["origin"]["path"], "/docs");
        assert_eq!(values[0]["origin"]["filename"], "data.csv");
        assert_eq!(values[0]["origin"]["query"], "download=1");
        assert_eq!(values[0]["origin"]["fragment"], "row");
        handle.join().unwrap();
    }

    #[test]
    fn remote_http_json_split_preserves_origin_uri_components() {
        let (base_url, _requests, handle) = spawn_http_server(
            "200 OK",
            "application/json",
            r#"{"hits":[{"name":"alpha"},{"name":"beta"}]}"#,
        );
        let uri = UriRef::parse(format!("{base_url}/docs/data.json?download=1#hits").to_string())
            .unwrap();
        let authority = uri.authority().unwrap().as_str().to_string();
        let values = collect_values(
            fetch_remote_input_with_client_and_split(
                uri,
                &Client::builder().build().unwrap(),
                Some(SplitPath::parse("/hits/").unwrap()),
            )
            .unwrap(),
        );

        assert_eq!(values.len(), 2);
        assert!(values.iter().any(|value| value["name"] == "alpha"));
        assert!(values.iter().any(|value| value["name"] == "beta"));
        for value in values {
            assert_eq!(value["origin"]["scheme"], "http");
            assert_eq!(value["origin"]["authority"], authority);
            assert_eq!(value["origin"]["path"], "/docs");
            assert_eq!(value["origin"]["filename"], "data.json");
            assert_eq!(value["origin"]["query"], "download=1");
            assert_eq!(value["origin"]["fragment"], "hits");
        }
        handle.join().unwrap();
    }

    #[test]
    fn json_extension_is_accepted_for_local_input_detection() {
        let path = PathBuf::from("/tmp/example.json");
        let kind = local_input_kind(&path).unwrap();
        assert_eq!(kind, InputKind::Json);
    }

    #[test]
    fn remote_https_fetch_supports_extensionless_csv_and_sends_accept_header() {
        let (base_url, requests, handle) =
            spawn_https_server("200 OK", "text/csv", "name,count\nalpha,2\n");
        let client = test_https_client();
        let uri = UriRef::parse(format!("{base_url}/download").to_string()).unwrap();
        let authority = uri.authority().unwrap().as_str().to_string();

        let mut input = fetch_remote_input_with_client(uri, &client).unwrap();
        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
        assert_eq!(actual["name"], "alpha");
        assert_eq!(actual["count"], "2");
        assert_eq!(actual["origin"]["scheme"], "https");
        assert_eq!(actual["origin"]["authority"], authority);
        assert_eq!(actual["origin"]["path"], "/");
        assert_eq!(actual["origin"]["filename"], "download");
        assert!(actual["origin"].get("query").is_none());
        assert!(actual["origin"].get("fragment").is_none());

        let request = requests.recv().unwrap();
        let accept_header = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("accept")
                    .then(|| value.trim().to_string())
            })
            .unwrap_or_else(|| panic!("expected accept header in request: {request}"));
        let accept_values: Vec<&str> = accept_header.split(',').map(|value| value.trim()).collect();
        assert_eq!(
            accept_values,
            vec![
                "text/csv",
                "application/x-ndjson",
                "application/ndjson",
                "application/json",
                "application/toon",
                "application/x-toon",
                "text/toon",
            ]
        );

        handle.join().unwrap();
    }

    #[test]
    fn remote_https_fetch_supports_toon_extension() {
        let (base_url, _requests, handle) =
            spawn_https_server("200 OK", "application/octet-stream", "id: 1\nname: Alpha\n");
        let client = test_https_client();
        let uri =
            UriRef::parse(format!("{base_url}/events.toon?download=1#page").to_string()).unwrap();

        let values = collect_values(fetch_remote_input_with_client(uri, &client).unwrap());

        assert_eq!(values[0]["id"], 1);
        assert_eq!(values[0]["name"], "Alpha");
        assert_eq!(values[0]["origin"]["scheme"], "https");
        assert_eq!(values[0]["origin"]["path"], "/");
        assert_eq!(values[0]["origin"]["filename"], "events.toon");
        assert_eq!(values[0]["origin"]["query"], "download=1");
        assert_eq!(values[0]["origin"]["fragment"], "page");
        handle.join().unwrap();
    }

    #[test]
    fn remote_https_fetch_supports_toon_content_type() {
        let (base_url, _requests, handle) =
            spawn_https_server("200 OK", "text/toon", "id: 1\nname: Alpha\n");
        let client = test_https_client();
        let uri = UriRef::parse(format!("{base_url}/download").to_string()).unwrap();

        let values = collect_values(fetch_remote_input_with_client(uri, &client).unwrap());

        assert_eq!(values[0]["id"], 1);
        assert_eq!(values[0]["name"], "Alpha");
        assert_eq!(values[0]["origin"]["scheme"], "https");
        assert_eq!(values[0]["origin"]["path"], "/");
        assert_eq!(values[0]["origin"]["filename"], "download");
        handle.join().unwrap();
    }

    #[test]
    fn remote_https_fetch_fails_on_non_success_status() {
        let (base_url, _requests, handle) =
            spawn_https_server("404 Not Found", "text/plain", "missing");
        let client = test_https_client();
        let uri = UriRef::parse(format!("{base_url}/missing.ndjson").to_string()).unwrap();

        match fetch_remote_input_with_client(uri, &client) {
            Ok(_) => panic!("non-success status should fail"),
            Err(err) => assert!(err.to_string().contains("HTTP status 404")),
        }

        handle.join().unwrap();
    }

    #[test]
    fn remote_https_fetch_rejects_gzip_url_suffix() {
        let (base_url, _requests, handle) =
            spawn_https_server("200 OK", "application/octet-stream", "not really gzip");
        let client = test_https_client();
        let uri = UriRef::parse(format!("{base_url}/events.ndjson.gz").to_string()).unwrap();

        match fetch_remote_input_with_client(uri, &client) {
            Ok(_) => panic!("remote gzip input should fail"),
            Err(err) => assert!(
                err.to_string()
                    .contains("Unsupported remote gzip input format")
            ),
        }

        handle.join().unwrap();
    }

    #[test]
    fn remote_https_fetch_fails_on_transport_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let client = test_https_client();
        let uri = UriRef::parse(format!("https://localhost:{port}/missing.ndjson")).unwrap();

        match fetch_remote_input_with_client(uri, &client) {
            Ok(_) => panic!("transport failure should fail"),
            Err(err) => {
                let message = err.to_string();
                assert!(
                    message.contains("error sending request")
                        || message.contains("Connection refused")
                        || message.contains("tcp connect error"),
                    "unexpected transport error: {message}"
                );
            }
        }
    }

    fn test_https_client() -> Client {
        Client::builder()
            .https_only(true)
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap()
    }

    fn spawn_http_server(
        status: &str,
        content_type: &str,
        body: &str,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let status = status.to_string();
        let content_type = content_type.to_string();
        let body = body.to_string();
        let (tx, rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_http_request(stream, &tx, &status, &content_type, &body);
        });

        (format!("http://localhost:{port}"), rx, handle)
    }

    fn serve_http_request(
        mut stream: TcpStream,
        tx: &mpsc::Sender<String>,
        status: &str,
        content_type: &str,
        body: &str,
    ) {
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let count = stream.read(&mut buf).unwrap();
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buf[..count]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }

        tx.send(String::from_utf8(request).unwrap()).unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    fn spawn_https_server(
        status: &str,
        content_type: &str,
        body: &str,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = Arc::new(test_tls_config());
        let status = status.to_string();
        let content_type = content_type.to_string();
        let body = body.to_string();
        let (tx, rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let connection = ServerConnection::new(config).unwrap();
            let mut tls = StreamOwned::new(connection, stream);

            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let count = tls.read(&mut buf).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }

            tx.send(String::from_utf8(request).unwrap()).unwrap();

            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            tls.write_all(response.as_bytes()).unwrap();
            tls.flush().unwrap();
        });

        (format!("https://localhost:{port}"), rx, handle)
    }

    fn test_tls_config() -> ServerConfig {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_der: CertificateDer<'static> = certified.cert.der().clone();
        let key_der = PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
        let key_der: PrivateKeyDer<'static> = key_der.into();

        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .unwrap()
    }
}
