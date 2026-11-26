use super::{documents::read_file_documents, metadata::OriginMetadata};
use eyre::{Result, eyre};
use serde_json::value::RawValue;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
};

const MAX_FILE_DOCUMENT_WORKERS: usize = 8;

pub(super) type FileDocumentConverter =
    Arc<dyn Fn(&Path, &str, Option<&OriginMetadata>) -> Result<Vec<Box<RawValue>>> + Send + Sync>;

struct FileDocumentJob {
    path: PathBuf,
    origin: OriginMetadata,
}

pub(super) struct FileDocumentResult {
    worker_index: usize,
    pub(super) path: PathBuf,
    pub(super) documents: Result<Vec<Box<RawValue>>>,
}

struct FileDocumentWorker {
    sender: SyncSender<FileDocumentJob>,
    _handle: JoinHandle<()>,
}

pub(crate) struct FileDocumentWorkers {
    sources: Vec<(PathBuf, OriginMetadata)>,
    workers: Vec<FileDocumentWorker>,
    results: Receiver<FileDocumentResult>,
    idle_workers: VecDeque<usize>,
    next_job_index: usize,
    remaining_results: usize,
}

impl FileDocumentWorkers {
    pub(super) fn start(
        paths: Vec<PathBuf>,
        origins: Vec<OriginMetadata>,
        content_field: &str,
    ) -> Result<Self> {
        let available_workers = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        let worker_limit = available_workers.min(MAX_FILE_DOCUMENT_WORKERS);
        Self::start_with_converter(
            paths,
            origins,
            content_field,
            (worker_limit, Arc::new(read_file_documents)),
        )
    }

    pub(super) fn start_with_converter(
        paths: Vec<PathBuf>,
        origins: Vec<OriginMetadata>,
        content_field: &str,
        settings: (usize, FileDocumentConverter),
    ) -> Result<Self> {
        let (worker_limit, converter) = settings;
        let sources: Vec<_> = paths.into_iter().zip(origins).collect();
        let source_count = sources.len();
        let worker_count = worker_limit.max(1).min(sources.len());
        let (result_sender, results) = mpsc::sync_channel(worker_count);
        let mut workers = Vec::with_capacity(worker_count);

        for worker_index in 0..worker_count {
            let (sender, receiver) = mpsc::sync_channel::<FileDocumentJob>(1);
            let result_sender = result_sender.clone();
            let content_field = content_field.to_string();
            let converter = Arc::clone(&converter);
            let handle = thread::Builder::new()
                .name(format!("espipe-convert-{worker_index}"))
                .spawn(move || {
                    while let Ok(job) = receiver.recv() {
                        let documents =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                converter(&job.path, &content_field, Some(&job.origin))
                            }))
                            .unwrap_or_else(|_| Err(eyre!("file conversion worker panicked")));
                        let result = FileDocumentResult {
                            worker_index,
                            path: job.path,
                            documents,
                        };
                        if result_sender.send(result).is_err() {
                            break;
                        }
                    }
                })
                .map_err(|error| eyre!("Could not start file conversion worker: {error}"))?;
            workers.push(FileDocumentWorker {
                sender,
                _handle: handle,
            });
        }
        drop(result_sender);

        Ok(Self {
            sources,
            workers,
            results,
            idle_workers: (0..worker_count).collect(),
            next_job_index: 0,
            remaining_results: source_count,
        })
    }

    fn schedule_available(&mut self) -> Result<()> {
        while self.next_job_index < self.sources.len() {
            let Some(worker_index) = self.idle_workers.pop_front() else {
                break;
            };
            let (path, origin) = &self.sources[self.next_job_index];
            let job = FileDocumentJob {
                path: path.clone(),
                origin: origin.clone(),
            };
            self.workers[worker_index]
                .sender
                .send(job)
                .map_err(|_| eyre!("File conversion worker stopped unexpectedly"))?;
            self.next_job_index += 1;
        }
        Ok(())
    }

    pub(super) fn next_completed(&mut self) -> Result<Option<FileDocumentResult>> {
        if self.remaining_results == 0 {
            return Ok(None);
        }
        self.schedule_available()?;
        let result = self
            .results
            .recv()
            .map_err(|_| eyre!("File conversion workers stopped unexpectedly"))?;
        self.remaining_results -= 1;
        self.idle_workers.push_back(result.worker_index);
        self.schedule_available()?;
        Ok(Some(result))
    }
}
