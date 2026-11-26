use super::collect_values;
use crate::input::Input;
use crate::input::metadata::origin_from_uri;
use crate::input::remote::REMOTE_NDJSON_ERROR;
use crate::input::remote::fetch_remote_input_with_client;
use crate::input::remote::fetch_remote_input_with_client_and_split;
use crate::input::remote::validate_ndjson_file;
use crate::json_split::SplitPath;
use fluent_uri::UriRef;
use reqwest::blocking::Client;
use rustls::ServerConfig;
use rustls::ServerConnection;
use rustls::StreamOwned;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use rustls::pki_types::PrivatePkcs8KeyDer;
use serde_json::json;
use std::io::Cursor;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use tempfile::NamedTempFile;

#[test]
fn remote_streaming_input_does_not_report_a_local_file_count() {
    let remote = UriRef::parse("https://example.com/docs.ndjson".to_string()).unwrap();
    let input = Input::FileJson {
        source: remote.to_string(),
        reader: Box::new(std::io::BufReader::new(Box::new(Cursor::new(Vec::new())))),
        first_record: true,
        origin: Some(origin_from_uri(&remote)),
        file_identity: None,
        _temp_file: None,
    };

    assert_eq!(input.file_count(), None);
}

#[test]
fn json_validation_rejects_non_ndjson_payload() {
    let mut temp = NamedTempFile::new().unwrap();
    writeln!(temp, "\"hello\"").unwrap();

    let err = validate_ndjson_file(temp.as_file_mut()).unwrap_err();
    assert_eq!(err.to_string(), REMOTE_NDJSON_ERROR);
}

#[test]
fn json_validation_rejects_array_payload() {
    let mut temp = NamedTempFile::new().unwrap();
    writeln!(temp, "[1,2]").unwrap();

    let err = validate_ndjson_file(temp.as_file_mut()).unwrap_err();
    assert_eq!(err.to_string(), REMOTE_NDJSON_ERROR);
}

#[test]
fn remote_http_fetch_preserves_origin_uri_components() {
    let (base_url, _requests, handle) = spawn_server(
        None,
        http_response("200 OK", "text/csv", "name,count\nalpha,2\n"),
    );
    let uri =
        UriRef::parse(format!("{base_url}/docs/data.csv?download=1#row").to_string()).unwrap();
    let authority = uri.authority().unwrap().as_str().to_string();
    let values = collect_values(
        fetch_remote_input_with_client(uri, &Client::builder().build().unwrap()).unwrap(),
    );

    assert_eq!(values[0]["name"], "alpha");
    assert_eq!(
        values[0]["origin"],
        json!({
            "scheme": "http", "authority": authority, "path": "/docs",
            "filename": "data.csv", "query": "download=1", "fragment": "row"
        })
    );
    handle.join().unwrap();
}

#[test]
fn remote_http_json_split_preserves_origin_uri_components() {
    let (base_url, _requests, handle) = spawn_server(
        None,
        http_response(
            "200 OK",
            "application/json",
            r#"{"hits":[{"name":"alpha"},{"name":"beta"}]}"#,
        ),
    );
    let uri =
        UriRef::parse(format!("{base_url}/docs/data.json?download=1#hits").to_string()).unwrap();
    let authority = uri.authority().unwrap().as_str().to_string();
    let values = collect_values(
        fetch_remote_input_with_client_and_split(
            uri,
            &Client::builder().build().unwrap(),
            Some(SplitPath::parse("/hits/").unwrap()),
        )
        .unwrap(),
    );

    let mut names: Vec<_> = values
        .iter()
        .map(|value| value["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["alpha", "beta"]);
    let origin = json!({
        "scheme": "http", "authority": authority, "path": "/docs",
        "filename": "data.json", "query": "download=1", "fragment": "hits"
    });
    assert!(values.iter().all(|value| value["origin"] == origin));
    handle.join().unwrap();
}

#[test]
fn remote_https_fetch_supports_extensionless_csv_and_sends_accept_header() {
    let (base_url, requests, handle) = spawn_server(
        Some(test_tls_config()),
        http_response("200 OK", "text/csv", "name,count\nalpha,2\n"),
    );
    let client = test_https_client();
    let uri = UriRef::parse(format!("{base_url}/download").to_string()).unwrap();
    let authority = uri.authority().unwrap().as_str().to_string();

    let mut input = fetch_remote_input_with_client(uri, &client).unwrap();
    let mut line = String::new();
    let value = input.read_line(&mut line).unwrap();
    let actual: serde_json::Value = serde_json::from_str(value.get()).unwrap();
    assert_eq!(
        (&actual["name"], &actual["count"]),
        (&json!("alpha"), &json!("2"))
    );
    assert_eq!(
        actual["origin"],
        json!({
            "scheme": "https", "authority": authority, "path": "/", "filename": "download"
        })
    );

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
            "application/toon",
            "application/x-toon",
            "text/toon",
        ]
    );

    handle.join().unwrap();
}

#[test]
fn remote_https_fetch_detects_toon_by_extension_or_content_type() {
    for (content_type, path, filename, has_query) in [
        (
            "application/octet-stream",
            "/events.toon?download=1#page",
            "events.toon",
            true,
        ),
        ("text/toon", "/download", "download", false),
    ] {
        let (base_url, _requests, handle) = spawn_server(
            Some(test_tls_config()),
            http_response("200 OK", content_type, "id: 1\nname: Alpha\n"),
        );
        let uri = UriRef::parse(format!("{base_url}{path}")).unwrap();
        let values =
            collect_values(fetch_remote_input_with_client(uri, &test_https_client()).unwrap());

        let document = &values[0];
        assert_eq!(
            (&document["id"], &document["name"]),
            (&json!(1), &json!("Alpha"))
        );
        let origin = &document["origin"];
        assert_eq!(
            (&origin["scheme"], &origin["path"], &origin["filename"]),
            (&json!("https"), &json!("/"), &json!(filename)),
        );
        if has_query {
            assert_eq!(
                (&origin["query"], &origin["fragment"]),
                (&json!("download=1"), &json!("page"))
            );
        }
        handle.join().unwrap();
    }
}

#[test]
fn remote_https_fetch_fails_on_non_success_status() {
    let (base_url, _requests, handle) = spawn_server(
        Some(test_tls_config()),
        http_response("404 Not Found", "text/plain", "missing"),
    );
    let client = test_https_client();
    let uri = UriRef::parse(format!("{base_url}/missing.ndjson").to_string()).unwrap();

    match fetch_remote_input_with_client(uri, &client) {
        Ok(_) => panic!("non-success status should fail"),
        Err(err) => assert!(err.to_string().contains("HTTP status 404")),
    }

    handle.join().unwrap();
}

#[test]
fn remote_https_fetch_rejects_gzip_url_suffix() {
    let (base_url, _requests, handle) = spawn_server(
        Some(test_tls_config()),
        http_response("200 OK", "application/octet-stream", "not really gzip"),
    );
    let client = test_https_client();
    let uri = UriRef::parse(format!("{base_url}/events.ndjson.gz").to_string()).unwrap();

    match fetch_remote_input_with_client(uri, &client) {
        Ok(_) => panic!("remote gzip input should fail"),
        Err(err) => assert!(
            err.to_string()
                .contains("Unsupported remote gzip input format")
        ),
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

fn spawn_server(
    tls: Option<ServerConfig>,
    response: Vec<u8>,
) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let scheme = if tls.is_some() { "https" } else { "http" };
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        if let Some(config) = tls {
            let connection = ServerConnection::new(Arc::new(config)).unwrap();
            serve_http_request(StreamOwned::new(connection, stream), &tx, &response);
        } else {
            serve_http_request(stream, &tx, &response);
        }
    });
    (format!("{scheme}://localhost:{port}"), rx, handle)
}

fn serve_http_request(mut stream: impl Read + Write, tx: &mpsc::Sender<String>, response: &[u8]) {
    let mut request = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let count = stream.read(&mut buf).unwrap();
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buf[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    tx.send(String::from_utf8(request).unwrap()).unwrap();
    stream.write_all(response).unwrap();
    stream.flush().unwrap();
}

fn http_response(status: &str, content_type: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ).into_bytes()
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
