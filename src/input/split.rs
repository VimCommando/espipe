use super::{
    Input, InputDocument,
    discovery::{DiscoveryOptions, is_batch_import, resolve_file_document_paths_with_options},
    effective_generate_id, is_stdin, log_skipped_file,
    metadata::{
        DocumentDiscriminator, FileInputIdentity, OriginMetadata, finalize_file_input_document,
    },
    stream::{InputKind, local_file_reader, local_input_kind},
    validate_input_schemes,
};
use crate::json_split::{SplitDocument, SplitEvent, SplitPath, start_split_reader};
use eyre::{Result, eyre};
use fluent_uri::UriRef;
use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, stdin},
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
};
use tempfile::NamedTempFile;

pub(crate) struct SplitInput {
    pub(super) source: String,
    reader: SplitReader,
    metadata: SplitMetadata,
    finished: bool,
}

#[derive(Default)]
pub(super) struct SplitMetadata {
    pub(super) origin: Option<OriginMetadata>,
    pub(super) file_identity: Option<FileInputIdentity>,
    pub(super) _temp_file: Option<NamedTempFile>,
}

struct SplitReader {
    receiver: Receiver<SplitEvent>,
    pending: VecDeque<SplitDocument>,
}

enum SplitReadError {
    Parse(String),
    Disconnected,
}

impl std::fmt::Display for SplitReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(error) => f.write_str(error),
            Self::Disconnected => f.write_str("JSON split parser stopped unexpectedly"),
        }
    }
}

impl SplitReader {
    fn start(reader: Box<dyn Read + Send>, source: &str, split: SplitPath) -> Result<Self> {
        Ok(Self {
            receiver: start_split_reader(reader, source.to_owned(), split)?,
            pending: VecDeque::new(),
        })
    }

    fn next(&mut self) -> std::result::Result<Option<SplitDocument>, SplitReadError> {
        loop {
            if let Some(document) = self.pending.pop_front() {
                return Ok(Some(document));
            }
            match self.receiver.recv() {
                Ok(SplitEvent::Documents(documents)) => self.pending.extend(documents),
                Ok(SplitEvent::Complete) => return Ok(None),
                Ok(SplitEvent::Failure(error)) => return Err(SplitReadError::Parse(error)),
                Err(_) => return Err(SplitReadError::Disconnected),
            }
        }
    }
}

impl SplitInput {
    pub(super) fn read_document(&mut self) -> Result<InputDocument> {
        if self.finished {
            return Err(eyre!("No split document"));
        }
        match self.reader.next() {
            Ok(Some(document)) => finalize_split_document(
                document,
                self.metadata.origin.as_ref(),
                self.metadata.file_identity.as_mut(),
            ),
            Ok(None) => {
                self.finished = true;
                Err(eyre!("No split document"))
            }
            Err(error) => {
                self.finished = true;
                Err(match error {
                    SplitReadError::Parse(error) => eyre!(error),
                    error => eyre!("{}: {error}", self.source),
                })
            }
        }
    }

    pub(super) fn file_count(&self) -> Option<usize> {
        self.metadata
            .origin
            .as_ref()
            .filter(|origin| origin.scheme == "file")
            .map(|_| 1)
    }
}

pub(crate) struct LocalSplitDocuments {
    sources: std::vec::IntoIter<(PathBuf, OriginMetadata)>,
    split: SplitPath,
    generate_id: bool,
    skip_errors: bool,
    active: Option<ActiveLocalSplit>,
    pub(super) evaluated_documents: usize,
    pub(super) file_count: usize,
}

struct ActiveLocalSplit {
    path: PathBuf,
    reader: SplitReader,
    origin: OriginMetadata,
    identity: FileInputIdentity,
}

impl LocalSplitDocuments {
    pub(super) fn read_document(&mut self) -> Result<InputDocument> {
        loop {
            if self.active.is_none() {
                self.open_next()?;
            }
            if let Some(document) = self.read_active()? {
                self.evaluated_documents += 1;
                return Ok(document);
            }
        }
    }

    fn open_next(&mut self) -> Result<()> {
        let (path, origin) = self
            .sources
            .next()
            .ok_or_else(|| eyre!("No split document"))?;
        let result = self.open_source(&path, origin);
        match result {
            Ok(state) => self.active = Some(state),
            Err(error) if self.skip_errors => log_skipped_file(&path, error),
            Err(error) => return Err(error),
        }
        Ok(())
    }

    fn open_source(&self, path: &Path, origin: OriginMetadata) -> Result<ActiveLocalSplit> {
        let file = File::open(path)?;
        let source = path.display().to_string();
        let reader =
            SplitReader::start(local_file_reader(file, path), &source, self.split.clone())?;
        let identity = FileInputIdentity::new(path, self.generate_id)?;
        Ok(ActiveLocalSplit {
            path: path.to_owned(),
            reader,
            origin,
            identity,
        })
    }

    fn read_active(&mut self) -> Result<Option<InputDocument>> {
        let Some(state) = self.active.as_mut() else {
            return Ok(None);
        };
        match state.reader.next() {
            Ok(Some(document)) => {
                finalize_split_document(document, Some(&state.origin), Some(&mut state.identity))
                    .map(Some)
            }
            Ok(None) => {
                self.active = None;
                Ok(None)
            }
            Err(error) => {
                let state = self.active.take().expect("split source is active");
                if self.skip_errors {
                    log_skipped_file(&state.path, error);
                    Ok(None)
                } else {
                    Err(eyre!("{}: {error}", state.path.display()))
                }
            }
        }
    }
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

#[cfg(test)]
pub(super) fn open_split_inputs(
    uris: Vec<UriRef<String>>,
    split: SplitPath,
    generate_id: Option<bool>,
) -> Result<Input> {
    open_split_inputs_with_options(uris, split, generate_id, DiscoveryOptions::default())
}

pub(super) fn open_split_inputs_with_options(
    uris: Vec<UriRef<String>>,
    split: SplitPath,
    generate_id: Option<bool>,
    discovery_options: DiscoveryOptions,
) -> Result<Input> {
    validate_input_schemes(&uris, false)?;
    if is_stdin(&uris) {
        return open_json_split(
            Box::new(stdin()),
            "stdin".to_string(),
            split,
            SplitMetadata::default(),
        );
    }

    let skip_errors = is_batch_import(&uris);
    let (paths, origins) = resolve_file_document_paths_with_options(&uris, discovery_options)?;
    for path in &paths {
        if !matches!(local_input_kind(path)?, InputKind::Json) {
            return Err(eyre!("--split requires a JSON input source"));
        }
    }
    let effective_generate_id = effective_generate_id(generate_id, paths.len());
    if paths.len() == 1 && !skip_errors {
        let path = paths.into_iter().next().unwrap();
        let origin = origins.into_iter().next();
        return open_local_split_input(path, origin, split, effective_generate_id);
    }

    let file_count = paths.len();
    Ok(Input::LocalSplitDocuments(LocalSplitDocuments {
        sources: paths
            .into_iter()
            .zip(origins)
            .collect::<Vec<_>>()
            .into_iter(),
        split,
        generate_id: effective_generate_id,
        skip_errors,
        active: None,
        evaluated_documents: 0,
        file_count,
    }))
}

pub(super) fn open_json_split(
    reader: Box<dyn Read + Send>,
    source: String,
    split: SplitPath,
    metadata: SplitMetadata,
) -> Result<Input> {
    let reader = SplitReader::start(reader, &source, split)?;
    Ok(Input::JsonSplit(SplitInput {
        source,
        reader,
        metadata,
        finished: false,
    }))
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
        SplitMetadata {
            origin,
            file_identity,
            _temp_file: None,
        },
    )
}
