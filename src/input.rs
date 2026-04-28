use eyre::{Report, Result, eyre};
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
    Stdin {
        reader: Box<BufReader<Stdin>>,
    },
    FileDocuments {
        source: String,
        documents: Vec<Box<RawValue>>,
        index: usize,
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
            Input::Stdin { reader, .. } => read_json_line(reader, line_buffer, false),
            Input::FileDocuments {
                documents, index, ..
            } => {
                let Some(document) = documents.get(*index) else {
                    return Err(eyre!("No file document"));
                };
                *index += 1;
                RawValue::from_string(document.get().to_string()).map_err(Into::into)
            }
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
                    InputKind::Csv | InputKind::Ndjson => return open_local_file(path),
                    InputKind::Json if !should_use_file_document(&path) => {
                        return open_local_file(path);
                    }
                    InputKind::Json | InputKind::FileDocument => {}
                }
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
        Some(Err(_)) | None => Err(eyre!("No CSV record")),
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
                    .from_reader(Box::new(file) as Box<dyn Read + Send>),
            ),
            _temp_file: None,
        }),
        InputKind::Ndjson | InputKind::Json => Ok(Input::FileJson {
            source,
            reader: Box::new(BufReader::new(Box::new(file) as Box<dyn Read + Send>)),
            first_record: true,
            _temp_file: None,
        }),
        InputKind::FileDocument => open_file_documents(vec![source], "body"),
    }
}

fn open_file_documents(values: Vec<String>, content_field: &str) -> Result<Input> {
    let paths = resolve_file_document_paths(values)?;
    let include_file_metadata = paths.len() > 1;
    let mut documents = Vec::new();
    for path in &paths {
        documents.extend(read_file_documents(
            path,
            content_field,
            include_file_metadata,
        )?);
    }
    let source = format!("{} file document(s)", paths.len());
    Ok(Input::FileDocuments {
        source,
        documents,
        index: 0,
    })
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
            if path.is_file() {
                paths.insert(path);
            }
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
        Some("yml" | "yaml") => read_yaml_file_document(path, include_file_metadata),
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
    (None, text)
}

fn read_yaml_file_document(path: &Path, include_file_metadata: bool) -> Result<Vec<Box<RawValue>>> {
    let text = read_text_file(path)?;
    let content = yaml_mapping_to_json_map(&text)
        .map_err(|err| eyre!("{}: invalid YAML document shape: {err}", path.display()))?;
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
            "text/csv, application/x-ndjson, application/ndjson, application/json",
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
        InputKind::FileDocument => Err(eyre!("Unsupported remote input format")),
    }
}

fn remote_input_kind(uri: &UriRef<String>, response: &Response) -> Result<InputKind> {
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

    Err(eyre!("Unsupported remote input format"))
}

fn local_input_kind(path: &Path) -> Result<InputKind> {
    input_kind_from_path(path.to_string_lossy().as_ref())
        .ok_or_else(|| eyre!("Unsupported file extension"))
}

fn input_kind_from_path(path: &str) -> Option<InputKind> {
    let extension = PathBuf::from(path)
        .extension()
        .and_then(OsStr::to_str)?
        .to_ascii_lowercase();
    match extension.as_str() {
        "csv" => Some(InputKind::Csv),
        "ndjson" => Some(InputKind::Ndjson),
        "json" => Some(InputKind::Json),
        "md" | "markdown" | "txt" | "text" | "log" | "yml" | "yaml" | "jsonl" => {
            Some(InputKind::FileDocument)
        }
        _ => None,
    }
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
        fetch_remote_input_with_client, local_input_kind, open_input_values,
        validate_content_field, validate_ndjson_file,
    };
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

    fn temp_path(suffix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("espipe-input-{nanos}.{suffix}"))
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
        let err = input_err(open_input_values(vec![uri(&path)], "body"));
        assert!(err.contains("conflicts with content field 'body'"));
    }

    #[test]
    fn markdown_non_mapping_frontmatter_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "---\n- bad\n---\n# Body\n").unwrap();

        let err = input_err(open_input_values(vec![uri(&path)], "body"));

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
        let err = input_err(open_input_values(vec![uri(&path)], "body"));
        assert!(err.contains("invalid YAML document shape"));
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
        let err = input_err(open_input_values(vec![uri(&path), uri(&path)], "body"));
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
        let err = input_err(open_input_values(vec![uri(&path)], "body"));
        assert!(err.contains("JSON line must be an object"));
    }

    #[test]
    fn invalid_utf8_file_document_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.txt");
        fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();

        let err = input_err(open_input_values(vec![uri(&path)], "body"));

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
            ]
        );

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
