use eyre::{eyre, Report, Result};
use fluent_uri::UriRef;
use serde_json::Value;
use std::{
    ffi::OsStr,
    fs::File,
    io::{stdin, BufRead, BufReader, Stdin},
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
    pub fn read_line(&mut self, line_buffer: &mut String) -> Result<Value> {
        match self {
            Input::FileJson { reader, .. } => {
                reader.read_line(line_buffer)?;
                serde_json::from_str(line_buffer).map_err(|e| eyre!("Error parsing JSON: {e}"))
            }
            Input::FileCsv { reader, .. } => match reader.deserialize().next() {
                Some(record) => {
                    let record: CsvRecord = record?;
                    let json = serde_json::to_string(&record)?;
                    let value: Value = serde_json::from_str(&json)?;
                    Ok(value)
                }
                None => return Err(eyre!("No CSV record")),
            },
            Input::Stdin { reader, .. } => {
                let mut buffer = String::new();
                reader.read_line(&mut buffer)?;
                serde_json::from_str(&buffer).map_err(|e| eyre!("Error parsing JSON: {e}"))
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
