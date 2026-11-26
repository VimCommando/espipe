use super::InputDocument;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use eyre::{Result, eyre};
use fluent_uri::UriRef;
use serde_json::{Map, Value, value::RawValue};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

#[derive(Clone, Debug)]
pub(crate) struct OriginMetadata {
    pub(super) scheme: String,
    pub(super) authority: Option<String>,
    pub(super) path: String,
    query: Option<String>,
    fragment: Option<String>,
    filename: String,
}

#[derive(Clone, Debug)]
pub(crate) struct FileInputIdentity {
    pub(super) path: PathBuf,
    bundle_id: String,
    generate_id: bool,
    document_index: usize,
}

impl FileInputIdentity {
    pub(super) fn new(path: &Path, generate_id: bool) -> Result<Self> {
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

    pub(super) fn next_record_discriminator(&mut self) -> DocumentDiscriminator {
        let index = self.document_index;
        self.document_index += 1;
        DocumentDiscriminator::Record(index)
    }
}

#[derive(Clone, Debug)]
pub(super) enum DocumentDiscriminator {
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

pub(super) fn finalize_file_input_document(
    raw: Box<RawValue>,
    origin: Option<&OriginMetadata>,
    file_identity: Option<&mut FileInputIdentity>,
    discriminator: Option<DocumentDiscriminator>,
) -> Result<InputDocument> {
    let generated_id =
        match (file_identity, discriminator) {
            (Some(identity), Some(discriminator)) if identity.generate_id => Some(
                file_document_id(&identity.bundle_id, &identity.path, discriminator)?,
            ),
            _ => None,
        };
    let raw = add_origin_to_raw(raw, origin)?;
    Ok(InputDocument { raw, generated_id })
}

pub(super) fn base_file_document(origin: Option<&OriginMetadata>) -> Map<String, Value> {
    let mut document = Map::new();
    add_origin_metadata(&mut document, origin);
    document
}

pub(super) fn add_origin_metadata(
    document: &mut Map<String, Value>,
    origin: Option<&OriginMetadata>,
) {
    if let Some(origin) = origin {
        document.insert("origin".to_string(), origin.clone().into_value());
    }
}

pub(super) fn origin_from_local_path(path: &Path) -> OriginMetadata {
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

pub(super) fn normalize_local_path(path: &Path) -> Result<PathBuf> {
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

pub(super) fn relative_path_from_working_dir(path: &Path, working_dir: &Path) -> PathBuf {
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

static BUNDLE_IDENTIFIER: OnceLock<Result<String, String>> = OnceLock::new();

pub(super) fn bundle_identifier() -> Result<String> {
    BUNDLE_IDENTIFIER
        .get_or_init(|| resolve_bundle_identifier().map_err(|err| err.to_string()))
        .clone()
        .map_err(|message| eyre!("{message}"))
}

fn resolve_bundle_identifier() -> Result<String> {
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

pub(super) fn file_document_id(
    bundle_id: &str,
    path: &Path,
    discriminator: DocumentDiscriminator,
) -> Result<String> {
    file_document_id_for_relative_path(bundle_id, &working_relative_path(path), discriminator)
}

pub(super) fn file_document_id_for_relative_path(
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

pub(super) fn origin_from_uri(uri: &UriRef<String>) -> OriginMetadata {
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
    pub(super) fn into_value(self) -> Value {
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

pub(super) fn raw_documents(documents: Vec<Map<String, Value>>) -> Result<Vec<Box<RawValue>>> {
    documents
        .into_iter()
        .map(|document| {
            RawValue::from_string(Value::Object(document).to_string()).map_err(Into::into)
        })
        .collect()
}
