use eyre::{Report, Result, eyre};
use fluent_uri::UriRef;
use serde_json::value::RawValue;
use std::{
    ffi::OsStr,
    fs::File,
    io::{BufRead, BufReader, Stdin, stdin},
    path::PathBuf,
};

#[derive(Debug)]
pub enum Input {
    Url(UriRef<String>),
    FileJson {
        path: PathBuf,
        reader: Box<BufReader<File>>,
    },
    FileCsv {
        path: PathBuf,
        reader: Box<csv::Reader<File>>,
    },
    Stdin {
        reader: Box<BufReader<Stdin>>,
    },
}

type CsvRecord = std::collections::HashMap<String, String>;

impl Input {
    pub fn read_line(&mut self, line_buffer: &mut String) -> Result<Box<RawValue>> {
        match self {
            Input::FileJson { reader, .. } => {
                reader.read_line(line_buffer)?;
                if line_buffer.is_empty() {
                    return Err(eyre!("No JSON record"));
                }
                serde_json::from_str(line_buffer).map_err(|e| eyre!("Error parsing JSON: {e}"))
            }
            Input::FileCsv { reader, .. } => match reader.deserialize::<CsvRecord>().next() {
                Some(Ok(record)) => {
                    let json = serde_json::to_string(&record)?;
                    serde_json::value::RawValue::from_string(json).map_err(Into::into)
                }
                Some(Err(_)) | None => return Err(eyre!("No CSV record")),
            },
            Input::Stdin { reader, .. } => {
                reader.read_line(line_buffer)?;
                if line_buffer.is_empty() {
                    return Err(eyre!("No JSON record"));
                }
                serde_json::from_str(line_buffer).map_err(|e| eyre!("Error parsing JSON: {e}"))
            }
            Input::Url(_) => Err(eyre!("URL input handling not implemented")),
        }
    }
}

impl TryFrom<UriRef<String>> for Input {
    type Error = Report;

    fn try_from(uri: UriRef<String>) -> Result<Self, Self::Error> {
        log::trace!("{uri:?}");
        let open_file = |str| {
            let path = PathBuf::from(str);
            let file = File::open(&path)?;
            match path.extension().and_then(OsStr::to_str) {
                Some("csv") => {
                    let reader = Box::new(
                        csv::ReaderBuilder::new()
                            .has_headers(true)
                            .from_reader(file),
                    );
                    Ok(Input::FileCsv { path, reader })
                }
                Some("ndjson") => {
                    let reader = Box::new(BufReader::new(file));
                    Ok(Input::FileJson { path, reader })
                }
                _ => Err(eyre!("Unsupported file extension")),
            }
        };

        let path_str = uri.path().as_str();
        log::debug!("{path_str}");
        match uri.scheme() {
            Some(scheme) if ["http", "https"].contains(&scheme.as_str()) => Ok(Input::Url(uri)),
            Some(scheme) if scheme.as_str() == "file" => open_file(path_str),
            Some(scheme) => Err(eyre!("Unsupported input scheme: {scheme}")),
            None => match path_str {
                "-" => Ok(Input::Stdin {
                    reader: Box::new(BufReader::new(stdin())),
                }),
                _ => open_file(path_str),
            },
        }
    }
}

impl std::fmt::Display for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Input::Url(uri) => write!(f, "{uri}"),
            Input::FileJson { path, .. } => write!(f, "{}", path.display()),
            Input::FileCsv { path, .. } => write!(f, "{}", path.display()),
            Input::Stdin { .. } => write!(f, "stdin"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Input;
    use std::{
        fs::{self, File},
        io::BufReader,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

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
        let file = File::open(&path).unwrap();
        let mut input = Input::FileJson {
            path: path.clone(),
            reader: Box::new(BufReader::new(file)),
        };

        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        assert_eq!(value.get(), "{\"a\":1}");

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_line_converts_csv_to_raw_json() {
        let path = temp_path("csv");
        fs::write(&path, "name,count\nalpha,2\n").unwrap();
        let file = File::open(&path).unwrap();
        let mut input = Input::FileCsv {
            path: path.clone(),
            reader: Box::new(csv::ReaderBuilder::new().has_headers(true).from_reader(file)),
        };

        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
        let expected = serde_json::json!({"name":"alpha","count":"2"});
        assert_eq!(actual, expected);

        fs::remove_file(path).unwrap();
    }
}
