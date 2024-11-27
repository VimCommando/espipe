use super::Sender;

use eyre::Result;
use serde_json::Value;
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Debug)]
pub struct FileOutput {
    writer: Arc<Mutex<BufWriter<File>>>,
    filename: String,
}

impl Sender for FileOutput {
    async fn send(&mut self, value: &Value) -> Result<usize> {
        let mut guard = self.writer.lock().expect("Failed to get writer lock");
        serde_json::to_writer(&mut *guard, value)?;
        writeln!(&mut *guard)?;
        Ok(1)
    }

    async fn close(self) -> Result<usize> {
        Ok(0)
    }
}

impl TryFrom<PathBuf> for FileOutput {
    type Error = eyre::Report;

    fn try_from(path: PathBuf) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;
        let writer = Arc::new(Mutex::new(BufWriter::new(file)));
        let filename = path.to_string_lossy().to_string();
        Ok(Self { writer, filename })
    }
}

impl std::fmt::Display for FileOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.filename)
    }
}
