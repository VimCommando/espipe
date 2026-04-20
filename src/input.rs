use eyre::{Report, Result, eyre};
use fluent_uri::UriRef;
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT, CONTENT_TYPE},
};
use serde_json::value::RawValue;
use std::{
    ffi::OsStr,
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Stdin, Write, stdin},
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::{Builder, NamedTempFile};

pub enum Input {
    FileJson {
        source: String,
        reader: Box<BufReader<Box<dyn Read + Send>>>,
        _temp_file: Option<NamedTempFile>,
    },
    FileCsv {
        source: String,
        reader: Box<csv::Reader<Box<dyn Read + Send>>>,
        _temp_file: Option<NamedTempFile>,
    },
    Stdin {
        reader: Box<BufReader<Stdin>>,
    },
}

type CsvRecord = std::collections::HashMap<String, String>;
const REMOTE_NDJSON_ERROR: &str = "JSON payload does not look like required NDJSON input format.";
const JSON_LINE_OPENING_ERROR: &str =
    "Each record must be a JSON object or array starting with '{' or '['";
const REMOTE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputKind {
    Csv,
    Ndjson,
    Json,
}

impl Input {
    pub async fn try_new(uri: UriRef<String>) -> Result<Self> {
        log::trace!("{uri:?}");
        let path_str = uri.path().as_str();
        log::debug!("{path_str}");

        match uri.scheme().map(|scheme| scheme.as_str()) {
            Some("https") => tokio::task::spawn_blocking(move || fetch_remote_input(uri))
                .await
                .map_err(|err| eyre!("Remote input fetch task failed: {err}"))?,
            Some("http") => Err(eyre!("Unsupported input scheme: http")),
            Some("file") => open_local_file(PathBuf::from(path_str)),
            Some(scheme) => Err(eyre!("Unsupported input scheme: {scheme}")),
            None => match path_str {
                "-" => Ok(Input::Stdin {
                    reader: Box::new(BufReader::new(stdin())),
                }),
                _ => open_local_file(PathBuf::from(path_str)),
            },
        }
    }

    pub fn read_line(&mut self, line_buffer: &mut String) -> Result<Box<RawValue>> {
        match self {
            Input::FileJson { reader, .. } => read_json_line(reader, line_buffer),
            Input::FileCsv { reader, .. } => read_csv_line(reader),
            Input::Stdin { reader, .. } => read_json_line(reader, line_buffer),
        }
    }
}

impl TryFrom<UriRef<String>> for Input {
    type Error = Report;

    fn try_from(uri: UriRef<String>) -> Result<Self, Self::Error> {
        let path_str = uri.path().as_str();

        match uri.scheme().map(|scheme| scheme.as_str()) {
            Some("https") => fetch_remote_input(uri),
            Some("http") => Err(eyre!("Unsupported input scheme: http")),
            Some("file") => open_local_file(PathBuf::from(path_str)),
            Some(scheme) => Err(eyre!("Unsupported input scheme: {scheme}")),
            None => match path_str {
                "-" => Ok(Input::Stdin {
                    reader: Box::new(BufReader::new(stdin())),
                }),
                _ => open_local_file(PathBuf::from(path_str)),
            },
        }
    }
}

impl std::fmt::Display for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Input::FileJson { source, .. } => write!(f, "{source}"),
            Input::FileCsv { source, .. } => write!(f, "{source}"),
            Input::Stdin { .. } => write!(f, "stdin"),
        }
    }
}

fn read_json_line<R: BufRead>(reader: &mut R, line_buffer: &mut String) -> Result<Box<RawValue>> {
    reader.read_line(line_buffer)?;
    if line_buffer.is_empty() {
        return Err(eyre!("No JSON record"));
    }
    let raw: Box<RawValue> =
        serde_json::from_str(line_buffer).map_err(|e| eyre!("Error parsing JSON: {e}"))?;
    ensure_json_opening(raw.get(), JSON_LINE_OPENING_ERROR)?;
    Ok(raw)
}

fn read_csv_line(reader: &mut csv::Reader<Box<dyn Read + Send>>) -> Result<Box<RawValue>> {
    match reader.deserialize::<CsvRecord>().next() {
        Some(Ok(record)) => {
            let json = serde_json::to_string(&record)?;
            serde_json::value::RawValue::from_string(json).map_err(Into::into)
        }
        Some(Err(_)) | None => Err(eyre!("No CSV record")),
    }
}

fn open_local_file(path: PathBuf) -> Result<Input> {
    let source = path.display().to_string();
    let file = File::open(&path)?;
    match local_input_kind(&path)? {
        InputKind::Csv => Ok(Input::FileCsv {
            source,
            reader: Box::new(
                csv::ReaderBuilder::new()
                    .has_headers(true)
                    .from_reader(Box::new(file) as Box<dyn Read + Send>),
            ),
            _temp_file: None,
        }),
        InputKind::Ndjson | InputKind::Json => Ok(Input::FileJson {
            source,
            reader: Box::new(BufReader::new(Box::new(file) as Box<dyn Read + Send>)),
            _temp_file: None,
        }),
    }
}

fn fetch_remote_input(uri: UriRef<String>) -> Result<Input> {
    let client = Client::builder()
        .https_only(true)
        .connect_timeout(REMOTE_CONNECT_TIMEOUT)
        .timeout(REMOTE_REQUEST_TIMEOUT)
        .build()?;
    fetch_remote_input_with_client(uri, &client)
}

fn fetch_remote_input_with_client(uri: UriRef<String>, client: &Client) -> Result<Input> {
    let mut response = client
        .get(uri.as_str())
        .header(
            ACCEPT,
            "text/csv, application/x-ndjson, application/ndjson, application/json",
        )
        .send()?;

    if !response.status().is_success() {
        return Err(eyre!(
            "Remote fetch failed with HTTP status {}",
            response.status()
        ));
    }

    let kind = remote_input_kind(&uri, &response)?;
    let suffix = match kind {
        InputKind::Csv => ".csv",
        InputKind::Ndjson => ".ndjson",
        InputKind::Json => ".json",
    };

    let mut temp_file = Builder::new().suffix(suffix).tempfile()?;
    std::io::copy(&mut response, temp_file.as_file_mut())?;
    temp_file.as_file_mut().flush()?;

    if kind == InputKind::Json {
        validate_ndjson_file(temp_file.as_file_mut())?;
    }

    let reader_file = temp_file.reopen()?;
    let source = uri.to_string();

    match kind {
        InputKind::Csv => Ok(Input::FileCsv {
            source,
            reader: Box::new(
                csv::ReaderBuilder::new()
                    .has_headers(true)
                    .from_reader(Box::new(reader_file) as Box<dyn Read + Send>),
            ),
            _temp_file: Some(temp_file),
        }),
        InputKind::Ndjson | InputKind::Json => Ok(Input::FileJson {
            source,
            reader: Box::new(BufReader::new(Box::new(reader_file) as Box<dyn Read + Send>)),
            _temp_file: Some(temp_file),
        }),
    }
}

fn remote_input_kind(uri: &UriRef<String>, response: &Response) -> Result<InputKind> {
    if let Some(kind) = input_kind_from_path(uri.path().as_str()) {
        return Ok(kind);
    }

    let Some(content_type) = response.headers().get(CONTENT_TYPE) else {
        return Err(eyre!("Unsupported remote input format"));
    };
    let content_type = content_type.to_str()?.to_ascii_lowercase();

    if content_type.contains("text/csv") || content_type.contains("application/csv") {
        return Ok(InputKind::Csv);
    }
    if content_type.contains("application/x-ndjson") || content_type.contains("application/ndjson")
    {
        return Ok(InputKind::Ndjson);
    }
    if content_type.contains("application/json") || content_type.ends_with("+json") {
        return Ok(InputKind::Json);
    }

    Err(eyre!("Unsupported remote input format"))
}

fn local_input_kind(path: &Path) -> Result<InputKind> {
    input_kind_from_path(path.to_string_lossy().as_ref()).ok_or_else(|| eyre!("Unsupported file extension"))
}

fn input_kind_from_path(path: &str) -> Option<InputKind> {
    let extension = PathBuf::from(path)
        .extension()
        .and_then(OsStr::to_str)?
        .to_ascii_lowercase();
    match extension.as_str() {
        "csv" => Some(InputKind::Csv),
        "ndjson" => Some(InputKind::Ndjson),
        "json" => Some(InputKind::Json),
        _ => None,
    }
}

fn validate_ndjson_file(file: &mut File) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut reader = BufReader::new(&mut *file);
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }

        let raw: Box<RawValue> = serde_json::from_str(&line).map_err(|_| eyre!(REMOTE_NDJSON_ERROR))?;
        ensure_json_opening(raw.get(), REMOTE_NDJSON_ERROR)?;
    }

    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

fn ensure_json_opening(input: &str, error_message: &str) -> Result<()> {
    match input.bytes().find(|byte| !byte.is_ascii_whitespace()) {
        Some(b'{') | Some(b'[') => Ok(()),
        _ => Err(eyre!(error_message.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Input, InputKind, REMOTE_NDJSON_ERROR, fetch_remote_input_with_client, local_input_kind,
        validate_ndjson_file,
    };
    use fluent_uri::UriRef;
    use reqwest::blocking::Client;
    use rustls::{
        ServerConfig, ServerConnection, StreamOwned,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        sync::{Arc, mpsc},
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };
    use tempfile::NamedTempFile;

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
        let mut input = Input::try_from(UriRef::parse(path.to_string_lossy().into_owned()).unwrap()).unwrap();

        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        assert_eq!(value.get(), "{\"a\":1}");

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_line_converts_csv_to_raw_json() {
        let path = temp_path("csv");
        fs::write(&path, "name,count\nalpha,2\n").unwrap();
        let mut input = Input::try_from(UriRef::parse(path.to_string_lossy().into_owned()).unwrap()).unwrap();

        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
        let expected = serde_json::json!({"name":"alpha","count":"2"});
        assert_eq!(actual, expected);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn json_validation_rejects_non_ndjson_payload() {
        let mut temp = NamedTempFile::new().unwrap();
        writeln!(temp, "\"hello\"").unwrap();

        let err = validate_ndjson_file(temp.as_file_mut()).unwrap_err();
        assert_eq!(err.to_string(), REMOTE_NDJSON_ERROR);
    }

    #[test]
    fn http_input_scheme_is_rejected() {
        let uri = UriRef::parse("http://example.com/data.ndjson".to_string()).unwrap();
        match Input::try_from(uri) {
            Ok(_) => panic!("http input should be rejected"),
            Err(err) => assert!(err.to_string().contains("Unsupported input scheme: http")),
        }
    }

    #[test]
    fn json_extension_is_accepted_for_local_input_detection() {
        let path = PathBuf::from("/tmp/example.json");
        let kind = local_input_kind(&path).unwrap();
        assert_eq!(kind, InputKind::Json);
    }

    #[test]
    fn remote_https_fetch_supports_extensionless_csv_and_sends_accept_header() {
        let (base_url, requests, handle) =
            spawn_https_server("200 OK", "text/csv", "name,count\nalpha,2\n");
        let client = test_https_client();
        let uri = UriRef::parse(format!("{base_url}/download").to_string()).unwrap();

        let mut input = fetch_remote_input_with_client(uri, &client).unwrap();
        let mut line = String::new();
        let value = input.read_line(&mut line).unwrap();
        let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
        assert_eq!(actual, serde_json::json!({"name":"alpha","count":"2"}));

        let request = requests.recv().unwrap();
        let accept_header = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("accept")
                    .then(|| value.trim().to_string())
            })
            .unwrap_or_else(|| panic!("expected accept header in request: {request}"));
        let accept_values: Vec<&str> = accept_header.split(',').map(|value| value.trim()).collect();
        assert_eq!(
            accept_values,
            vec![
                "text/csv",
                "application/x-ndjson",
                "application/ndjson",
                "application/json",
            ]
        );

        handle.join().unwrap();
    }

    #[test]
    fn remote_https_fetch_fails_on_non_success_status() {
        let (base_url, _requests, handle) =
            spawn_https_server("404 Not Found", "text/plain", "missing");
        let client = test_https_client();
        let uri = UriRef::parse(format!("{base_url}/missing.ndjson").to_string()).unwrap();

        match fetch_remote_input_with_client(uri, &client) {
            Ok(_) => panic!("non-success status should fail"),
            Err(err) => assert!(err.to_string().contains("HTTP status 404")),
        }

        handle.join().unwrap();
    }

    #[test]
    fn remote_https_fetch_fails_on_transport_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let client = test_https_client();
        let uri = UriRef::parse(format!("https://localhost:{port}/missing.ndjson")).unwrap();

        match fetch_remote_input_with_client(uri, &client) {
            Ok(_) => panic!("transport failure should fail"),
            Err(err) => {
                let message = err.to_string();
                assert!(
                    message.contains("error sending request")
                        || message.contains("Connection refused")
                        || message.contains("tcp connect error"),
                    "unexpected transport error: {message}"
                );
            }
        }
    }

    fn test_https_client() -> Client {
        Client::builder()
            .https_only(true)
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap()
    }

    fn spawn_https_server(
        status: &str,
        content_type: &str,
        body: &str,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = Arc::new(test_tls_config());
        let status = status.to_string();
        let content_type = content_type.to_string();
        let body = body.to_string();
        let (tx, rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let connection = ServerConnection::new(config).unwrap();
            let mut tls = StreamOwned::new(connection, stream);

            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let count = tls.read(&mut buf).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }

            tx.send(String::from_utf8(request).unwrap()).unwrap();

            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            tls.write_all(response.as_bytes()).unwrap();
            tls.flush().unwrap();
        });

        (format!("https://localhost:{port}"), rx, handle)
    }

    fn test_tls_config() -> ServerConfig {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_der: CertificateDer<'static> = certified.cert.der().clone();
        let key_der = PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
        let key_der: PrivateKeyDer<'static> = key_der.into();

        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .unwrap()
    }
}
