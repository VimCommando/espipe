use super::Sender;

use eyre::Result;
use serde_json::value::RawValue;
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
    async fn send(&mut self, value: Box<RawValue>) -> Result<usize> {
        let mut guard = self.writer.lock().expect("Failed to get writer lock");
        guard.write_all(value.get().as_bytes())?;
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

#[cfg(test)]
mod tests {
    use super::{FileOutput, Sender};
    use serde_json::value::RawValue;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("espipe-output-{nanos}.ndjson"))
    }

    #[tokio::test]
    async fn file_output_writes_raw_json_directly() {
        let path = temp_path();
        let mut output = FileOutput::try_from(path.clone()).unwrap();

        output
            .send(RawValue::from_string("{\"a\":1}".to_string()).unwrap())
            .await
            .unwrap();
        output.close().await.unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":1}\n");
        fs::remove_file(path).unwrap();
    }
}
