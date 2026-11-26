use super::{
    Input,
    documents::open_file_documents,
    metadata::{FileInputIdentity, OriginMetadata, origin_from_local_path},
};
use eyre::{Result, eyre};
use flate2::read::GzDecoder;
use fluent_uri::UriRef;
use serde_json::{Map, Value, value::RawValue};
use std::{
    ffi::OsStr,
    fs::File,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

use tempfile::NamedTempFile;

type CsvRecord = std::collections::HashMap<String, String>;
pub(super) const JSON_LINE_OPENING_ERROR: &str =
    "Each record must be a JSON object starting with '{'";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InputKind {
    Csv,
    Ndjson,
    Json,
    Toon,
    FileDocument,
}

pub(super) fn read_json_line<R: BufRead>(
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
    }
    let raw: Box<RawValue> =
        serde_json::from_str(line_buffer).map_err(|e| eyre!("Error parsing JSON: {e}"))?;
    ensure_json_opening(raw.get(), JSON_LINE_OPENING_ERROR)?;
    Ok(raw)
}

pub(super) fn read_csv_line(
    reader: &mut csv::Reader<Box<dyn Read + Send>>,
) -> Result<Box<RawValue>> {
    match reader.deserialize::<CsvRecord>().next() {
        Some(Ok(record)) => {
            let json = serde_json::to_string(&record)?;
            serde_json::value::RawValue::from_string(json).map_err(Into::into)
        }
        Some(Err(err)) => Err(err.into()),
        None => Err(eyre!("No CSV record")),
    }
}

#[derive(Default)]
pub(crate) struct ToonState {
    pending: String,
    document_index: usize,
    buffered_rows: Vec<Value>,
    eof: bool,
}

impl ToonState {
    pub(super) fn read(
        &mut self,
        source: &str,
        reader: &mut impl BufRead,
    ) -> Result<Box<RawValue>> {
        if let Some(row) = self.buffered_rows.pop() {
            return toon_row_value_to_raw(source, self.document_index, row);
        }
        if self.eof {
            return Err(eyre!("No Toon document"));
        }
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                self.eof = true;
                return self.decode_pending(source);
            }
            if line.trim() != "---" {
                self.pending.push_str(&line);
                continue;
            }
            if !self.pending.trim().is_empty() {
                return self.decode_pending(source);
            }
        }
    }

    fn decode_pending(&mut self, source: &str) -> Result<Box<RawValue>> {
        if self.pending.trim().is_empty() {
            return Err(eyre!("No Toon document"));
        }
        self.document_index += 1;
        let document = decode_toon_documents(
            source,
            self.document_index,
            &self.pending,
            &mut self.buffered_rows,
        )?;
        self.pending.clear();
        Ok(document)
    }
}

pub(super) struct StreamSource {
    pub(super) source: String,
    pub(super) origin: Option<OriginMetadata>,
    pub(super) file_identity: Option<FileInputIdentity>,
    pub(super) temp_file: Option<NamedTempFile>,
}

pub(super) fn open_stream(
    kind: InputKind,
    reader: Box<dyn Read + Send>,
    source: StreamSource,
) -> Result<Input> {
    let StreamSource {
        source,
        origin,
        file_identity,
        temp_file,
    } = source;
    match kind {
        InputKind::Csv => Ok(Input::FileCsv {
            source,
            reader: Box::new(
                csv::ReaderBuilder::new()
                    .has_headers(true)
                    .from_reader(reader),
            ),
            origin,
            file_identity,
            _temp_file: temp_file,
        }),
        InputKind::Ndjson | InputKind::Json => Ok(Input::FileJson {
            source,
            reader: Box::new(BufReader::new(reader)),
            first_record: true,
            origin,
            file_identity,
            _temp_file: temp_file,
        }),
        InputKind::Toon => Ok(Input::FileToon {
            source,
            reader: Box::new(BufReader::new(reader)),
            state: ToonState::default(),
            origin,
            file_identity,
            _temp_file: temp_file,
        }),
        InputKind::FileDocument => Err(eyre!("Unsupported remote input format")),
    }
}

pub(super) fn open_local_file(path: PathBuf, generate_id: bool) -> Result<Input> {
    let source = path.display().to_string();
    let file = File::open(&path)?;
    let kind = local_input_kind(&path)?;
    if kind == InputKind::FileDocument {
        return open_file_documents(
            vec![UriRef::parse(source).map_err(|err| eyre!("Invalid local file URI: {err:?}"))?],
            "body",
            generate_id,
        );
    }
    let file_identity = FileInputIdentity::new(&path, generate_id)?;
    let origin = Some(origin_from_local_path(&file_identity.path));
    open_stream(
        kind,
        local_file_reader(file, &path),
        StreamSource {
            source,
            origin,
            file_identity: Some(file_identity),
            temp_file: None,
        },
    )
}

pub(super) fn local_input_kind(path: &Path) -> Result<InputKind> {
    input_kind_from_path(path.to_string_lossy().as_ref())
        .ok_or_else(|| eyre!("Unsupported file extension"))
}

pub(super) fn input_kind_from_path(path: &str) -> Option<InputKind> {
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

pub(super) fn local_file_reader(file: File, path: &Path) -> Box<dyn Read + Send> {
    if has_path_suffix(path.to_string_lossy().as_ref(), ".gz") {
        return Box::new(GzDecoder::new(file));
    }
    Box::new(file)
}

pub(super) fn has_path_suffix(path: &str, suffix: &str) -> bool {
    path.len() >= suffix.len()
        && path
            .get(path.len() - suffix.len()..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
}

pub(super) fn is_compressed_input(path: &str) -> bool {
    has_path_suffix(path, ".gz")
}

pub(super) fn is_unsupported_compressed_input(path: &str) -> bool {
    is_compressed_input(path)
        && !has_path_suffix(path, ".csv.gz")
        && !has_path_suffix(path, ".ndjson.gz")
}

pub(super) fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
}

pub(super) fn ensure_json_opening(input: &str, error_message: &str) -> Result<()> {
    match input.bytes().find(|byte| !byte.is_ascii_whitespace()) {
        Some(b'{') => Ok(()),
        _ => Err(eyre!(error_message.to_string())),
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
