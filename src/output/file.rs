use super::Sender;

use eyre::Result;
use flate2::{Compression, write::GzEncoder};
use serde_json::value::RawValue;
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Debug)]
pub struct FileOutput {
    writer: Arc<Mutex<FileWriter>>,
    filename: String,
}

#[derive(Debug)]
enum FileWriter {
    Plain(BufWriter<File>),
    Gzip(GzEncoder<BufWriter<File>>),
}

impl FileWriter {
    fn finish(self) -> Result<()> {
        match self {
            FileWriter::Plain(mut writer) => writer.flush().map_err(Into::into),
            FileWriter::Gzip(writer) => writer.finish().map(|_| ()).map_err(Into::into),
        }
    }
}

impl Write for FileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            FileWriter::Plain(writer) => writer.write(buf),
            FileWriter::Gzip(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            FileWriter::Plain(writer) => writer.flush(),
            FileWriter::Gzip(writer) => writer.flush(),
        }
    }
}

impl Sender for FileOutput {
    async fn send(&mut self, value: Box<RawValue>) -> Result<usize> {
        let mut guard = self.writer.lock().expect("Failed to get writer lock");
        guard.write_all(value.get().as_bytes())?;
        writeln!(&mut *guard)?;
        Ok(1)
    }

    async fn close(self) -> Result<usize> {
        let writer = Arc::try_unwrap(self.writer)
            .map_err(|_| eyre::eyre!("File output writer is still shared"))?
            .into_inner()
            .expect("Failed to get writer lock");
        writer.finish()?;
        Ok(0)
    }
}

impl TryFrom<PathBuf> for FileOutput {
    type Error = eyre::Report;

    fn try_from(path: PathBuf) -> Result<Self> {
        if is_unsupported_gzip_output(&path) {
            return Err(eyre::eyre!(
                "Unsupported compressed output format: {}",
                path.display()
            ));
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;
        let writer = if is_gzip_ndjson_output(&path) {
            FileWriter::Gzip(GzEncoder::new(BufWriter::new(file), Compression::default()))
        } else {
            FileWriter::Plain(BufWriter::new(file))
        };
        let writer = Arc::new(Mutex::new(writer));
        let filename = path.to_string_lossy().to_string();
        Ok(Self { writer, filename })
    }
}

impl std::fmt::Display for FileOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.filename)
    }
}

fn is_gzip_ndjson_output(path: &PathBuf) -> bool {
    path.to_string_lossy()
        .to_ascii_lowercase()
        .ends_with(".ndjson.gz")
}

fn is_unsupported_gzip_output(path: &PathBuf) -> bool {
    let lower_path = path.to_string_lossy().to_ascii_lowercase();
    lower_path.ends_with(".gz") && !lower_path.ends_with(".ndjson.gz")
}

#[cfg(test)]
mod tests {
    use super::{FileOutput, Sender};
    use flate2::read::GzDecoder;
    use serde_json::value::RawValue;
    use std::{
        fs,
        io::Read,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_path(suffix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("espipe-output-{nanos}.{suffix}"))
    }

    #[tokio::test]
    async fn file_output_writes_raw_json_directly() {
        let path = temp_path("ndjson");
        let mut output = FileOutput::try_from(path.clone()).unwrap();

        output
            .send(RawValue::from_string("{\"a\":1}".to_string()).unwrap())
            .await
            .unwrap();
        output.close().await.unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":1}\n");
        fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn file_output_writes_gzip_ndjson() {
        let path = temp_path("ndjson.gz");
        let mut output = FileOutput::try_from(path.clone()).unwrap();

        output
            .send(RawValue::from_string("{\"a\":1}".to_string()).unwrap())
            .await
            .unwrap();
        output.close().await.unwrap();

        let file = fs::File::open(&path).unwrap();
        let mut decoder = GzDecoder::new(file);
        let mut contents = String::new();
        decoder.read_to_string(&mut contents).unwrap();
        assert_eq!(contents, "{\"a\":1}\n");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn file_output_rejects_unsupported_gzip_suffix_before_create() {
        let path = temp_path("csv.gz");

        let err = FileOutput::try_from(path.clone()).unwrap_err();

        assert!(
            err.to_string()
                .contains("Unsupported compressed output format")
        );
        assert!(!path.exists());
    }
}
