use eyre::{eyre, Report, Result};
use fluent_uri::UriRef;
use std::{
    fs::File,
    io::{stdin, BufRead, BufReader, Stdin},
    path::PathBuf,
};

#[derive(Debug)]
pub enum Input {
    Url(UriRef<String>),
    File(File, PathBuf),
    Stdin(Stdin),
}

impl Input {
    pub fn get_reader(self) -> Result<Box<dyn BufRead + Send>> {
        match self {
            Input::Url(_) => Err(eyre!("Url reader not implemented")),
            Input::File(file, _) => Ok(Box::new(BufReader::new(file.try_clone()?))),
            Input::Stdin(stdin) => Ok(Box::new(BufReader::new(stdin))),
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
            Ok(Input::File(file, path))
        };

        let path_str = uri.path().as_str();
        log::debug!("{path_str}");
        match uri.scheme() {
            Some(scheme) if ["http", "https"].contains(&scheme.as_str()) => Ok(Input::Url(uri)),
            Some(scheme) if scheme.as_str() == "file" => open_file(path_str),
            Some(scheme) => Err(eyre!("Unsupported input scheme: {scheme}")),
            None => match path_str {
                "-" => Ok(Input::Stdin(stdin())),
                _ => open_file(path_str),
            },
        }
    }
}

impl std::fmt::Display for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Input::Url(uri) => write!(f, "{uri}"),
            Input::File(_, path) => write!(f, "{}", path.display()),
            Input::Stdin(_) => write!(f, "stdin"),
        }
    }
}
