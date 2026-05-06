use eyre::{Report, Result, eyre};
use flate2::read::GzDecoder;
use fluent_uri::UriRef;
use glob::glob;
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT, CONTENT_TYPE},
};
use serde_json::{Map, Value, value::RawValue};
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Stdin, Write, stdin},
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::{Builder, NamedTempFile};

pub enum Input {
    FileJson {
        source: String,
        reader: Box<BufReader<Box<dyn Read + Send>>>,
        first_record: bool,
        _temp_file: Option<NamedTempFile>,
    },
    FileCsv {
        source: String,
        reader: Box<csv::Reader<Box<dyn Read + Send>>>,
        _temp_file: Option<NamedTempFile>,
    },
    FileToon {
        source: String,
        reader: Box<BufReader<Box<dyn Read + Send>>>,
        pending: String,
        document_index: usize,
        buffered: Vec<Box<RawValue>>,
        eof: bool,
        _temp_file: Option<NamedTempFile>,
    },
    Stdin {
        reader: Box<BufReader<Stdin>>,
    },
    FileDocuments {
        source: String,
        paths: Vec<PathBuf>,
        path_index: usize,
        documents: Vec<Box<RawValue>>,
        document_index: usize,
        content_field: String,
        include_file_metadata: bool,
    },
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
    pub async fn try_new(uris: Vec<UriRef<String>>, content_field: String) -> Result<Self> {
        validate_content_field(&content_field)?;
        if uris.is_empty() {
            return Err(eyre!("At least one input is required"));
        }
        if uris.len() == 1 {
            let uri = uris.into_iter().next().unwrap();
            return match uri.scheme().map(|scheme| scheme.as_str()) {
                Some("https") => tokio::task::spawn_blocking(move || fetch_remote_input(uri))
                    .await
                    .map_err(|err| eyre!("Remote input fetch task failed: {err}"))?,
                _ => open_input_values(vec![uri], &content_field),
            };
        }
        open_input_values(uris, &content_field)
    }

    pub fn read_line(&mut self, line_buffer: &mut String) -> Result<Box<RawValue>> {
        match self {
            Input::FileJson {
                reader,
                first_record,
                ..
            } => {
                let raw = read_json_line(reader, line_buffer, *first_record)?;
                *first_record = false;
                Ok(raw)
            }
            Input::FileCsv { reader, .. } => read_csv_line(reader),
            Input::FileToon {
                source,
                reader,
                pending,
                document_index,
                buffered,
                eof,
                ..
            } => read_toon_document(source, reader, pending, document_index, buffered, eof),
            Input::Stdin { reader, .. } => read_json_line(reader, line_buffer, false),
            Input::FileDocuments { .. } => read_file_document_line(self),
        }
    }

    pub fn read_next(&mut self, line_buffer: &mut String) -> Result<Option<Box<RawValue>>> {
        match self.read_line(line_buffer) {
            Ok(value) => Ok(Some(value)),
            Err(err) if is_end_of_input(&err) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

impl TryFrom<UriRef<String>> for Input {
    type Error = Report;

    fn try_from(uri: UriRef<String>) -> Result<Self, Self::Error> {
        match uri.scheme().map(|scheme| scheme.as_str()) {
            Some("https") => fetch_remote_input(uri),
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
    for uri in &uris {
        match uri.scheme().map(|scheme| scheme.as_str()) {
            Some("https") if uris.len() == 1 => return fetch_remote_input(uri.clone()),
            Some("https") => {
                return Err(eyre!("Remote inputs cannot be combined with file imports"));
            }
            Some("http") => return Err(eyre!("Unsupported input scheme: http")),
            Some("file") | None => {}
            Some(scheme) => return Err(eyre!("Unsupported input scheme: {scheme}")),
        }
    }

    if uris.len() == 1 {
        let uri = uris.into_iter().next().unwrap();
        let path_str = uri.path().as_str();
        if uri.scheme().is_none() && path_str == "-" {
            return Ok(Input::Stdin {
                reader: Box::new(BufReader::new(stdin())),
            });
        }
        let path = PathBuf::from(path_str);
        if !has_glob_metachar(path_str) {
            if let Ok(kind) = local_input_kind(&path) {
                match kind {
                    InputKind::Csv | InputKind::Ndjson | InputKind::Toon => {
                        return open_local_file(path);
                    }
                    InputKind::Json if !should_use_file_document(&path) => {
                        return open_local_file(path);
                    }
                    InputKind::Json | InputKind::FileDocument => {}
                }
            }
            if is_unsupported_compressed_input(path_str) {
                return Err(eyre!("Unsupported compressed input format: {path_str}"));
            }
        }
        return open_file_documents(vec![path_str.to_string()], content_field);
    }

    let values = uris
        .into_iter()
        .map(|uri| uri.path().as_str().to_string())
        .collect();
    open_file_documents(values, content_field)
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
    buffered: &mut Vec<Box<RawValue>>,
    eof: &mut bool,
) -> Result<Box<RawValue>> {
    if let Some(raw) = buffered.pop() {
        return Ok(raw);
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
            let raw = decode_toon_documents(source, *document_index, pending, buffered)?;
            pending.clear();
            return Ok(raw);
        }

        if line.trim() == "---" {
            if pending.trim().is_empty() {
                continue;
            }
            *document_index += 1;
            let raw = decode_toon_documents(source, *document_index, pending, buffered)?;
            pending.clear();
            return Ok(raw);
        }

        pending.push_str(&line);
    }
}

fn open_local_file(path: PathBuf) -> Result<Input> {
    let source = path.display().to_string();
    let file = File::open(&path)?;
    match local_input_kind(&path)? {
        InputKind::Csv => Ok(Input::FileCsv {
            source,
            reader: Box::new(
                csv::ReaderBuilder::new()
                    .has_headers(true)
                    .from_reader(local_file_reader(file, &path)),
            ),
            _temp_file: None,
        }),
        InputKind::Ndjson | InputKind::Json => Ok(Input::FileJson {
            source,
            reader: Box::new(BufReader::new(local_file_reader(file, &path))),
            first_record: true,
            _temp_file: None,
        }),
        InputKind::Toon => Ok(Input::FileToon {
            source,
            reader: Box::new(BufReader::new(local_file_reader(file, &path))),
            pending: String::new(),
            document_index: 0,
            buffered: Vec::new(),
            eof: false,
            _temp_file: None,
        }),
        InputKind::FileDocument => open_file_documents(vec![source], "body"),
    }
}

fn open_file_documents(values: Vec<String>, content_field: &str) -> Result<Input> {
    let paths = resolve_file_document_paths(values)?;
    let include_file_metadata = paths.len() > 1;
    let source = format!("{} file document(s)", paths.len());
    Ok(Input::FileDocuments {
        source,
        paths,
        path_index: 0,
        documents: Vec::new(),
        document_index: 0,
        content_field: content_field.to_string(),
        include_file_metadata,
    })
}

fn read_file_document_line(input: &mut Input) -> Result<Box<RawValue>> {
    let Input::FileDocuments {
        paths,
        path_index,
        documents,
        document_index,
        content_field,
        include_file_metadata,
        ..
    } = input
    else {
        return Err(eyre!("Input is not a file document import"));
    };

    loop {
        if let Some(document) = documents.get(*document_index) {
            *document_index += 1;
            return RawValue::from_string(document.get().to_string()).map_err(Into::into);
        }

        let Some(path) = paths.get(*path_index) else {
            return Err(eyre!("No file document"));
        };
        *path_index += 1;
        *documents = read_file_documents(path, content_field, *include_file_metadata)?;
        *document_index = 0;
    }
}

fn resolve_file_document_paths(values: Vec<String>) -> Result<Vec<PathBuf>> {
    let mut paths = BTreeSet::new();
    let mut any_glob = false;
    for value in values {
        if has_glob_metachar(&value) {
            any_glob = true;
            let mut matched_regular_file = false;
            for entry in glob(&value).map_err(|err| eyre!("Invalid glob pattern {value}: {err}"))? {
                let path = entry.map_err(|err| eyre!("Error expanding glob {value}: {err}"))?;
                if path.is_file() {
                    matched_regular_file = true;
                    paths.insert(path);
                }
            }
            if !matched_regular_file {
                return Err(eyre!("Glob matched no regular files: {value}"));
            }
        } else {
            let path = PathBuf::from(value);
            if !path.exists() {
                return Err(eyre!("File input does not exist: {}", path.display()));
            }
            if !path.is_file() {
                return Err(eyre!(
                    "File input is not a regular file: {}",
                    path.display()
                ));
            }
            paths.insert(path);
        }
    }
    for path in &paths {
        let path_str = path.to_string_lossy();
        if is_compressed_input(path_str.as_ref()) {
            return Err(eyre!("Unsupported compressed input format: {path_str}"));
        }
    }
    if paths.is_empty() {
        let kind = if any_glob {
            "glob inputs"
        } else {
            "file inputs"
        };
        return Err(eyre!("No regular files resolved from {kind}"));
    }
    Ok(paths.into_iter().collect())
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
    include_file_metadata: bool,
) -> Result<Vec<Box<RawValue>>> {
    match extension(path).as_deref() {
        Some("ndjson" | "jsonl") => read_ndjson_file_documents(path, include_file_metadata),
        Some("json") => read_json_file_document(path, include_file_metadata),
        Some("toon") => read_toon_file_documents(path, include_file_metadata),
        Some("yml" | "yaml") => read_yaml_file_document(path, content_field, include_file_metadata),
        Some("md" | "markdown") => {
            read_markdown_file_document(path, content_field, include_file_metadata)
        }
        _ => read_text_file_document(path, content_field, include_file_metadata),
    }
}

fn read_text_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|err| eyre!("{}: {err}", path.display()))?;
    String::from_utf8(bytes).map_err(|_| eyre!("{}: file is not valid UTF-8 text", path.display()))
}

fn read_text_file_document(
    path: &Path,
    content_field: &str,
    include_file_metadata: bool,
) -> Result<Vec<Box<RawValue>>> {
    let text = read_text_file(path)?;
    let mut document = base_file_document(path, include_file_metadata);
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
    include_file_metadata: bool,
) -> Result<Vec<Box<RawValue>>> {
    let text = read_text_file(path)?;
    let (frontmatter, body) = split_markdown_frontmatter(&text);
    let mut content = Map::new();
    if let Some(frontmatter) = frontmatter {
        content = yaml_mapping_to_json_map(frontmatter)
            .map_err(|err| eyre!("{}: invalid frontmatter: {err}", path.display()))?;
        if content.contains_key(content_field) {
            return Err(eyre!(
                "{}: frontmatter field conflicts with content field '{content_field}'",
                path.display()
            ));
        }
    }
    content.insert(content_field.to_string(), Value::String(body.to_string()));
    let mut document = base_file_document(path, include_file_metadata);
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
        "No JSON record" | "No CSV record" | "No file document" | "No Toon document"
    )
}

fn read_yaml_file_document(
    path: &Path,
    content_field: &str,
    include_file_metadata: bool,
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
    let mut document = base_file_document(path, include_file_metadata);
    document.insert("content".to_string(), Value::Object(content));
    raw_documents(vec![document])
}

fn yaml_mapping_to_json_map(text: &str) -> Result<Map<String, Value>> {
    let yaml: serde_yaml::Value = serde_yaml::from_str(text)?;
    let Value::Object(map) = serde_json::to_value(yaml)? else {
        return Err(eyre!("root must be a mapping"));
    };
    Ok(map)
}

fn read_json_file_document(path: &Path, include_file_metadata: bool) -> Result<Vec<Box<RawValue>>> {
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
    add_file_metadata(&mut document, path, include_file_metadata);
    raw_documents(vec![document])
}

fn read_ndjson_file_documents(
    path: &Path,
    include_file_metadata: bool,
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
        add_file_metadata(&mut document, path, include_file_metadata);
        docs.push(RawValue::from_string(Value::Object(document).to_string())?);
    }
    Ok(docs)
}

fn read_toon_file_documents(
    path: &Path,
    include_file_metadata: bool,
) -> Result<Vec<Box<RawValue>>> {
    let file = File::open(path).map_err(|err| eyre!("{}: {err}", path.display()))?;
    let mut reader = BufReader::new(Box::new(file) as Box<dyn Read + Send>);
    let mut pending = String::new();
    let mut document_index = 0;
    let mut buffered = Vec::new();
    let mut eof = false;
    let mut docs = Vec::new();
    let source = path.display().to_string();

    loop {
        match read_toon_document(
            &source,
            &mut reader,
            &mut pending,
            &mut document_index,
            &mut buffered,
            &mut eof,
        ) {
            Ok(mut raw) => {
                if include_file_metadata {
                    let mut document: Map<String, Value> = serde_json::from_str(raw.get())?;
                    add_file_metadata(&mut document, path, include_file_metadata);
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
    buffered: &mut Vec<Box<RawValue>>,
) -> Result<Box<RawValue>> {
    let value: Value = toon_format::decode_default(input).map_err(|err| {
        eyre!("{source}: document {document_index}: invalid Toon document: {err}")
    })?;
    let mut documents = toon_value_to_documents(source, document_index, value)?;
    documents.reverse();
    let Some(first) = documents.pop() else {
        return Err(eyre!(
            "{source}: document {document_index}: Toon document produced no rows"
        ));
    };
    buffered.extend(documents);
    Ok(first)
}

fn toon_value_to_documents(
    source: &str,
    document_index: usize,
    value: Value,
) -> Result<Vec<Box<RawValue>>> {
    let Value::Object(document) = value else {
        return Err(eyre!(
            "{source}: document {document_index}: Toon document must be an object"
        ));
    };

    if document.len() == 1 {
        let (_, value) = document.iter().next().unwrap();
        if let Value::Array(rows) = value {
            return rows
                .iter()
                .enumerate()
                .map(|(row_index, row)| {
                    let Value::Object(row) = row else {
                        return Err(eyre!(
                            "{source}: document {document_index} row {}: Toon array row must be an object",
                            row_index + 1
                        ));
                    };
                    RawValue::from_string(Value::Object(row.clone()).to_string()).map_err(Into::into)
                })
                .collect();
        }
    }

    RawValue::from_string(Value::Object(document).to_string())
        .map(|raw| vec![raw])
        .map_err(Into::into)
}

fn base_file_document(path: &Path, include_file_metadata: bool) -> Map<String, Value> {
    let mut document = Map::new();
    add_file_metadata(&mut document, path, include_file_metadata);
    document
}

fn add_file_metadata(document: &mut Map<String, Value>, path: &Path, include_file_metadata: bool) {
    if !include_file_metadata {
        return;
    }
    document.insert(
        "file".to_string(),
        Value::Object(Map::from_iter([
            (
                "path".to_string(),
                Value::String(path.display().to_string()),
            ),
            (
                "name".to_string(),
                Value::String(
                    path.file_name()
                        .and_then(OsStr::to_str)
                        .unwrap_or_default()
                        .to_string(),
                ),
            ),
        ])),
    );
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
        .https_only(true)
        .connect_timeout(REMOTE_CONNECT_TIMEOUT)
        .timeout(REMOTE_REQUEST_TIMEOUT)
        .build()?;
    fetch_remote_input_with_client(uri, &client)
}

fn fetch_remote_input_with_client(uri: UriRef<String>, client: &Client) -> Result<Input> {
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

    if kind == InputKind::Json {
        validate_ndjson_file(temp_file.as_file_mut())?;
    }

    let reader_file = temp_file.reopen()?;
    let source = uri.to_string();

    match kind {
        InputKind::Csv => Ok(Input::FileCsv {
            source,
            reader: Box::new(
                csv::ReaderBuilder::new()
                    .has_headers(true)
                    .from_reader(Box::new(reader_file) as Box<dyn Read + Send>),
            ),
            _temp_file: Some(temp_file),
        }),
        InputKind::Ndjson | InputKind::Json => Ok(Input::FileJson {
            source,
            reader: Box::new(BufReader::new(Box::new(reader_file) as Box<dyn Read + Send>)),
            first_record: true,
            _temp_file: Some(temp_file),
        }),
        InputKind::Toon => Ok(Input::FileToon {
            source,
            reader: Box::new(BufReader::new(Box::new(reader_file) as Box<dyn Read + Send>)),
            pending: String::new(),
            document_index: 0,
            buffered: Vec::new(),
            eof: false,
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
        Input, InputKind, JSON_LINE_OPENING_ERROR, REMOTE_NDJSON_ERROR,
        fetch_remote_input_with_client, input_kind_from_path, local_input_kind, open_input_values,
        validate_content_field, validate_ndjson_file,
    };
    use flate2::{Compression, write::GzEncoder};
    use fluent_uri::UriRef;
    use reqwest::blocking::Client;
    use rustls::{
        ServerConfig, ServerConnection, StreamOwned,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
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

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
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
    fn read_line_preserves_ndjson_as_raw_value() {
        let path = temp_path("ndjson");
        fs::write(&path, "{\"a\":1}\n").unwrap();
        let mut input =
            Input::try_from(UriRef::parse(path.to_string_lossy().into_owned()).unwrap()).unwrap();

        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        assert_eq!(value.get(), "{\"a\":1}");

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
        let expected = serde_json::json!({"name":"alpha","count":"2"});
        assert_eq!(actual, expected);

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
        let expected = serde_json::json!({"name":"alpha","count":"2"});
        assert_eq!(actual, expected);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_line_preserves_gzip_ndjson_as_raw_value() {
        let path = temp_path("ndjson.gz");
        write_gzip(&path, "{\"a\":1}\n");
        let mut input =
            Input::try_from(UriRef::parse(path.to_string_lossy().into_owned()).unwrap()).unwrap();

        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        assert_eq!(value.get(), "{\"a\":1}");

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
        let dir = tempfile::tempdir().unwrap();
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
        let dir = tempfile::tempdir().unwrap();
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

        assert_eq!(
            values,
            vec![serde_json::json!({"content":{"body":"# Title\nBody\n"}})]
        );
    }

    #[test]
    fn shell_expanded_files_are_sorted_deduplicated_and_include_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("b.txt");
        let a = dir.path().join("a.txt");
        fs::write(&b, "bravo").unwrap();
        fs::write(&a, "alpha").unwrap();

        let input = open_input_values(vec![uri(&b), uri(&a), uri(&a)], "body").unwrap();
        let values = collect_values(input);

        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["content"]["body"], "alpha");
        assert_eq!(values[1]["content"]["body"], "bravo");
        assert_eq!(values[0]["file"]["name"], "a.txt");
        assert_eq!(values[1]["file"]["name"], "b.txt");
    }

    #[test]
    fn recursive_glob_imports_regular_files_and_filters_directories() {
        let dir = tempfile::tempdir().unwrap();
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

        assert_eq!(
            values,
            vec![serde_json::json!({"content":{"markdown":"hello"}})]
        );
    }

    #[test]
    fn single_direct_file_document_omits_file_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        fs::write(&path, "hello").unwrap();

        let values = collect_values(open_input_values(vec![uri(&path)], "body").unwrap());

        assert!(values[0].get("file").is_none());
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

        assert_eq!(
            values,
            vec![serde_json::json!({"content":{"count":2,"title":"Hello"}})]
        );

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
        let dir = tempfile::tempdir().unwrap();
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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.json");
        fs::write(&path, "{\"a\":1}").unwrap();

        let values =
            collect_values(open_input_values(vec![uri(&path), uri(&path)], "body").unwrap());
        assert_eq!(values, vec![serde_json::json!({"a":1})]);

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
        assert_eq!(
            values,
            vec![serde_json::json!({"a":1}), serde_json::json!({"b":2})]
        );

        fs::write(&path, "[1,2]\n").unwrap();
        let err = read_err(open_input_values(vec![uri(&path)], "body"));
        assert!(err.contains("JSON line must be an object"));
    }

    #[test]
    fn toon_file_streams_object_documents_in_order() {
        let values = collect_values(Input::try_from(uri(&fixture_path("multi.toon"))).unwrap());

        assert_eq!(
            values,
            vec![
                serde_json::json!({"id":1,"name":"Alpha"}),
                serde_json::json!({"id":2,"name":"Bravo","tags":["search","bulk"]}),
            ]
        );
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

        assert_eq!(
            values,
            vec![serde_json::json!({"active":true,"id":1,"name":"Alpha"})]
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
        assert_eq!(actual, serde_json::json!({"id":1}));

        line.clear();
        let err = input.read_line(&mut line).unwrap_err().to_string();
        assert!(err.contains("invalid Toon document"));

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn toon_file_in_multi_input_includes_file_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("a.txt");
        let toon = dir.path().join("b.toon");
        fs::write(&text, "alpha").unwrap();
        fs::write(&toon, "id: 2\nname: Bravo\n").unwrap();

        let values =
            collect_values(open_input_values(vec![uri(&text), uri(&toon)], "body").unwrap());

        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["content"]["body"], "alpha");
        assert_eq!(values[1]["id"], 2);
        assert_eq!(values[1]["file"]["name"], "b.toon");
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
        assert_eq!(first.get(), "{\"a\":1}");
        line.clear();
        let second = input.read_line(&mut line).unwrap();
        assert_eq!(second.get(), "{\"b\":2}");

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
        assert_eq!(value.get(), "{\"a\":1}");

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
        assert_eq!(actual, serde_json::json!({"a":1,"b":{"c":2}}));

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
    fn http_input_scheme_is_rejected() {
        let uri = UriRef::parse("http://example.com/data.ndjson".to_string()).unwrap();
        match Input::try_from(uri) {
            Ok(_) => panic!("http input should be rejected"),
            Err(err) => assert!(err.to_string().contains("Unsupported input scheme: http")),
        }
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

        let mut input = fetch_remote_input_with_client(uri, &client).unwrap();
        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
        assert_eq!(actual, serde_json::json!({"name":"alpha","count":"2"}));

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
        let uri = UriRef::parse(format!("{base_url}/events.toon").to_string()).unwrap();

        let values = collect_values(fetch_remote_input_with_client(uri, &client).unwrap());

        assert_eq!(values, vec![serde_json::json!({"id":1,"name":"Alpha"})]);
        handle.join().unwrap();
    }

    #[test]
    fn remote_https_fetch_supports_toon_content_type() {
        let (base_url, _requests, handle) =
            spawn_https_server("200 OK", "text/toon", "id: 1\nname: Alpha\n");
        let client = test_https_client();
        let uri = UriRef::parse(format!("{base_url}/download").to_string()).unwrap();

        let values = collect_values(fetch_remote_input_with_client(uri, &client).unwrap());

        assert_eq!(values, vec![serde_json::json!({"id":1,"name":"Alpha"})]);
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
