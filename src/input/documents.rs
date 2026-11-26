use super::{
    Input, InputDocument,
    discovery::{DiscoveryOptions, resolve_file_document_paths_with_options},
    is_end_of_input, log_skipped_file,
    metadata::{
        DocumentDiscriminator, OriginMetadata, add_origin_metadata, base_file_document,
        bundle_identifier, file_document_id, raw_documents,
    },
    stream::{InputKind, ToonState, extension, local_input_kind, open_local_file},
    workers::{FileDocumentConverter, FileDocumentWorkers},
    yaml::{yaml_frontmatter_to_json_map, yaml_mapping_to_json_map},
};
use eyre::{Report, Result, eyre};
use fluent_uri::UriRef;
use serde_json::{Map, Value, value::RawValue};
use std::{
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
};

pub(super) struct FileDocumentOptions<'a> {
    pub(super) content_field: &'a str,
    pub(super) generate_id: bool,
    pub(super) skip_errors: bool,
}

pub(super) fn open_file_documents(
    values: Vec<UriRef<String>>,
    content_field: &str,
    generate_id: bool,
) -> Result<Input> {
    let (paths, origins) =
        resolve_file_document_paths_with_options(&values, DiscoveryOptions::default())?;
    open_file_documents_from_paths(
        paths,
        origins,
        FileDocumentOptions {
            content_field,
            generate_id,
            skip_errors: false,
        },
    )
}

pub(super) fn open_file_documents_from_paths(
    paths: Vec<PathBuf>,
    origins: Vec<OriginMetadata>,
    options: FileDocumentOptions<'_>,
) -> Result<Input> {
    open_file_documents_from_paths_with_converter(paths, origins, options, None)
}

pub(super) fn open_file_documents_from_paths_with_converter(
    paths: Vec<PathBuf>,
    origins: Vec<OriginMetadata>,
    options: FileDocumentOptions<'_>,
    worker_settings: Option<(usize, FileDocumentConverter)>,
) -> Result<Input> {
    let file_count = paths.len();
    let source = format!("{file_count} file document(s)");
    let workers = match worker_settings {
        Some(settings) => FileDocumentWorkers::start_with_converter(
            paths,
            origins,
            options.content_field,
            settings,
        )?,
        None => FileDocumentWorkers::start(paths, origins, options.content_field)?,
    };
    Ok(Input::FileDocuments {
        source,
        workers,
        current_path: None,
        documents: Vec::new(),
        document_index: 0,
        skip_errors: options.skip_errors,
        bundle_id: options.generate_id.then(bundle_identifier).transpose()?,
        evaluated_documents: 0,
        file_count,
    })
}

pub(super) fn read_file_document_line(input: &mut Input) -> Result<InputDocument> {
    let Input::FileDocuments {
        workers,
        current_path,
        documents,
        document_index,
        skip_errors,
        bundle_id,
        evaluated_documents,
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
            let generated_id = match bundle_id.as_deref() {
                Some(bundle_id) => converted_document_id(
                    &raw,
                    bundle_id,
                    current_path.as_deref(),
                    document_index_value,
                )?,
                None => None,
            };
            return Ok(InputDocument { raw, generated_id });
        }

        let Some(result) = workers.next_completed()? else {
            return Err(eyre!("No file document"));
        };
        *current_path = Some(result.path.clone());
        *documents = match result.documents {
            Ok(documents) => {
                *evaluated_documents += documents.len();
                documents
            }
            Err(error) if *skip_errors => {
                log_skipped_file(&result.path, error);
                Vec::new()
            }
            Err(error) => return Err(error),
        };
        *document_index = 0;
    }
}

fn converted_document_id(
    raw: &RawValue,
    bundle_id: &str,
    path: Option<&Path>,
    index: usize,
) -> Result<Option<String>> {
    let has_explicit_id = serde_json::from_str::<Value>(raw.get())
        .ok()
        .and_then(|value| value.as_object().map(|object| object.contains_key("_id")))
        .unwrap_or(false);
    if has_explicit_id {
        return Ok(None);
    }
    let path = path.ok_or_else(|| eyre!("File document path cursor is invalid"))?;
    file_document_id(bundle_id, path, DocumentDiscriminator::Record(index)).map(Some)
}

pub(super) fn read_local_file_document_line(
    input: &mut Input,
    line_buffer: &mut String,
) -> Result<InputDocument> {
    let Input::LocalFileDocuments {
        path,
        generate_id,
        active,
        complete,
        evaluated_documents,
    } = input
    else {
        return Err(eyre!("Input is not a local file document import"));
    };
    if !*complete {
        *complete = true;
        match open_local_file(path.clone(), *generate_id) {
            Ok(input) => *active = Some(Box::new(input)),
            Err(error) => log_skipped_file(path, error),
        }
    }
    let Some(current) = active.as_mut() else {
        return Err(eyre!("No file document"));
    };
    match current.read_next(line_buffer) {
        Ok(Some(document)) => {
            *evaluated_documents += 1;
            return Ok(document);
        }
        Ok(None) => {}
        Err(error) => log_skipped_file(path, error),
    }
    *active = None;
    Err(eyre!("No file document"))
}

pub(super) fn should_use_file_document(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("md" | "markdown" | "txt" | "text" | "log" | "yml" | "yaml" | "jsonl")
    )
}

pub(super) fn is_streaming_local_input(path: &Path) -> bool {
    match local_input_kind(path) {
        Ok(InputKind::Csv | InputKind::Ndjson | InputKind::Toon) => true,
        Ok(InputKind::Json) => !should_use_file_document(path),
        Ok(InputKind::FileDocument) | Err(_) => false,
    }
}

pub(super) fn read_file_documents(
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
    let mut reader = BufReader::new(file);
    let mut toon = ToonState::default();
    let mut docs = Vec::new();
    let source = path.display().to_string();

    loop {
        match toon.read(&source, &mut reader) {
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
