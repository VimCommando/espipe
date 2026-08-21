use eyre::{Result, eyre};
use serde::de::{self, DeserializeSeed, Error as _, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Value, value::RawValue};
use std::{
    fmt,
    io::{BufReader, Read},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    thread,
};

const SPLIT_BATCH_SIZE: usize = 128;
const QUEUED_BATCHES_PER_WORKER: usize = 2;
const SPLIT_READER_CAPACITY: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SplitPath {
    display: String,
    tokens: Vec<String>,
}

impl SplitPath {
    pub(crate) fn parse(input: &str) -> Result<Self> {
        if input.is_empty() || input == "/" {
            return Ok(Self {
                display: "/".to_string(),
                tokens: Vec::new(),
            });
        }
        if !input.starts_with('/') {
            return Err(eyre!(
                "Invalid --split path '{input}': JSON Pointer paths must start with '/'"
            ));
        }

        let normalized = input.strip_suffix('/').unwrap_or(input);
        if normalized.is_empty() || normalized == "/" {
            return Err(eyre!(
                "Invalid --split path '{input}': use '/' to split the root collection"
            ));
        }
        if normalized.ends_with('/') {
            return Err(eyre!(
                "Invalid --split path '{input}': final empty-name members are not supported; remove the extra trailing slash"
            ));
        }

        let tokens = normalized[1..]
            .split('/')
            .map(|token| decode_token(input, token))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            display: input.to_string(),
            tokens,
        })
    }

    pub(crate) fn display(&self) -> &str {
        &self.display
    }
}

fn decode_token(path: &str, token: &str) -> Result<String> {
    let mut decoded = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(character) = chars.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        match chars.next() {
            Some('0') => decoded.push('~'),
            Some('1') => decoded.push('/'),
            Some(other) => {
                return Err(eyre!(
                    "Invalid --split path '{path}': unsupported escape '~{other}'"
                ));
            }
            None => {
                return Err(eyre!(
                    "Invalid --split path '{path}': incomplete '~' escape"
                ));
            }
        }
    }
    Ok(decoded)
}

pub(crate) enum SplitEvent {
    Documents(Vec<Box<RawValue>>),
    Failure(String),
    Complete,
}

enum PendingDocument {
    Map { key: String, raw: Box<RawValue> },
    Array { index: usize, raw: Box<RawValue> },
}

pub(crate) fn start_split_reader<R>(
    reader: R,
    source: String,
    path: SplitPath,
) -> Result<Receiver<SplitEvent>>
where
    R: Read + Send + 'static,
{
    start_split_reader_with_worker_count(reader, source, path, split_worker_count())
}

fn start_split_reader_with_worker_count<R>(
    reader: R,
    source: String,
    path: SplitPath,
    worker_count: usize,
) -> Result<Receiver<SplitEvent>>
where
    R: Read + Send + 'static,
{
    debug_assert!(worker_count > 0);
    let result_capacity = worker_count
        .saturating_mul(QUEUED_BATCHES_PER_WORKER)
        .max(1);
    let (sender, receiver) = sync_channel(result_capacity);
    let parser_source = source.clone();
    thread::Builder::new()
        .name("espipe-json-split".to_string())
        .spawn(move || run_split_reader(reader, parser_source, path, sender, worker_count))
        .map_err(|error| eyre!("Could not start JSON split parser for {source}: {error}"))?;
    Ok(receiver)
}

fn split_worker_count() -> usize {
    thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .saturating_sub(1)
        .max(1)
}

fn run_split_reader<R>(
    reader: R,
    source: String,
    path: SplitPath,
    sender: SyncSender<SplitEvent>,
    worker_count: usize,
) where
    R: Read,
{
    let queue_capacity = worker_count
        .saturating_mul(QUEUED_BATCHES_PER_WORKER)
        .max(1);
    let (batch_sender, batch_receiver) = sync_channel(queue_capacity);
    let batch_receiver = Arc::new(Mutex::new(batch_receiver));
    let cancelled = Arc::new(AtomicBool::new(false));
    let context = Arc::new(format!(
        "{source}: error splitting JSON at '{}'",
        path.display()
    ));

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let batch_receiver = Arc::clone(&batch_receiver);
            let sender = sender.clone();
            let cancelled = Arc::clone(&cancelled);
            let context = Arc::clone(&context);
            scope.spawn(move || run_transform_worker(batch_receiver, sender, cancelled, context));
        }

        let reader = BufReader::with_capacity(SPLIT_READER_CAPACITY, reader);
        let mut deserializer = serde_json::Deserializer::from_reader(reader);
        let result = NavigateSeed {
            tokens: &path.tokens,
            full_path: path.display(),
            sender: &batch_sender,
            cancelled: &cancelled,
        }
        .deserialize(&mut deserializer)
        .and_then(|()| deserializer.end());

        if let Err(error) = result
            && !cancelled.load(Ordering::Acquire)
        {
            publish_failure(&sender, &cancelled, format!("{context}: {error}"));
        }
        drop(batch_sender);
    });

    if !cancelled.load(Ordering::Acquire) {
        let _ = sender.send(SplitEvent::Complete);
    }
}

fn run_transform_worker(
    receiver: Arc<Mutex<Receiver<Vec<PendingDocument>>>>,
    sender: SyncSender<SplitEvent>,
    cancelled: Arc<AtomicBool>,
    context: Arc<String>,
) {
    loop {
        let batch = match receiver.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => return,
        };
        let Ok(batch) = batch else {
            return;
        };

        if cancelled.load(Ordering::Acquire) {
            continue;
        }

        match transform_batch(batch) {
            Ok(documents) => {
                if sender.send(SplitEvent::Documents(documents)).is_err() {
                    cancelled.store(true, Ordering::Release);
                }
            }
            Err(error) => publish_failure(&sender, &cancelled, format!("{context}: {error}")),
        }
    }
}

fn publish_failure(sender: &SyncSender<SplitEvent>, cancelled: &AtomicBool, error: String) {
    if cancelled
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        let _ = sender.send(SplitEvent::Failure(error));
    }
}

fn transform_batch(batch: Vec<PendingDocument>) -> std::result::Result<Vec<Box<RawValue>>, String> {
    batch.into_iter().map(transform_document).collect()
}

fn transform_document(document: PendingDocument) -> std::result::Result<Box<RawValue>, String> {
    let (context, key, raw) = match document {
        PendingDocument::Map { key, raw } => {
            let context = format!("object property '{key}'");
            (context, Some(key), raw)
        }
        PendingDocument::Array { index, raw } => (format!("array element {index}"), None, raw),
    };

    let value = serde_json::from_str::<Value>(raw.get())
        .map_err(|error| format!("{context} could not be deserialized: {error}"))?;
    let Value::Object(mut object) = value else {
        return Err(format!("{context} must contain a JSON object document"));
    };

    if let Some(key) = key {
        if object.contains_key("id") {
            return Err(format!("{context} conflicts with generated 'id' field"));
        }
        object.insert("id".to_string(), Value::String(key));
    }

    let json = serde_json::to_string(&Value::Object(object))
        .map_err(|error| format!("could not serialize {context}: {error}"))?;
    RawValue::from_string(json).map_err(|error| format!("could not create {context}: {error}"))
}

struct NavigateSeed<'a> {
    tokens: &'a [String],
    full_path: &'a str,
    sender: &'a SyncSender<Vec<PendingDocument>>,
    cancelled: &'a AtomicBool,
}

impl<'de> DeserializeSeed<'de> for NavigateSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        if self.tokens.is_empty() {
            return deserializer.deserialize_any(SplitVisitor {
                full_path: self.full_path,
                sender: self.sender,
                cancelled: self.cancelled,
            });
        }
        deserializer.deserialize_any(NavigateVisitor {
            tokens: self.tokens,
            full_path: self.full_path,
            sender: self.sender,
            cancelled: self.cancelled,
        })
    }
}

struct NavigateVisitor<'a> {
    tokens: &'a [String],
    full_path: &'a str,
    sender: &'a SyncSender<Vec<PendingDocument>>,
    cancelled: &'a AtomicBool,
}

impl<'de> Visitor<'de> for NavigateVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "an object or array while resolving token '{}' in split path '{}'",
            self.tokens[0], self.full_path
        )
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let token = &self.tokens[0];
        let mut found = false;
        while let Some(key) = map.next_key::<String>()? {
            if !found && key == *token {
                map.next_value_seed(NavigateSeed {
                    tokens: &self.tokens[1..],
                    full_path: self.full_path,
                    sender: self.sender,
                    cancelled: self.cancelled,
                })?;
                found = true;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        if found {
            Ok(())
        } else {
            Err(A::Error::custom(format!(
                "split path '{}' did not resolve object token '{token}'",
                self.full_path
            )))
        }
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let token = &self.tokens[0];
        let target = parse_array_index(token).map_err(A::Error::custom)?;
        let mut index = 0usize;
        let mut found = false;
        loop {
            let has_element = if index == target {
                sequence
                    .next_element_seed(NavigateSeed {
                        tokens: &self.tokens[1..],
                        full_path: self.full_path,
                        sender: self.sender,
                        cancelled: self.cancelled,
                    })?
                    .is_some()
            } else {
                sequence.next_element::<IgnoredAny>()?.is_some()
            };
            if !has_element {
                break;
            }
            if index == target {
                found = true;
            }
            index += 1;
        }
        if found {
            Ok(())
        } else {
            Err(A::Error::custom(format!(
                "split path '{}' did not resolve array index '{token}'",
                self.full_path
            )))
        }
    }
}

fn parse_array_index(token: &str) -> std::result::Result<usize, String> {
    if token.is_empty()
        || (token.len() > 1 && token.starts_with('0'))
        || !token.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!(
            "split path array token '{token}' must be a canonical zero-based index"
        ));
    }
    token
        .parse::<usize>()
        .map_err(|_| format!("split path array index '{token}' is too large for this platform"))
}

struct SplitVisitor<'a> {
    full_path: &'a str,
    sender: &'a SyncSender<Vec<PendingDocument>>,
    cancelled: &'a AtomicBool,
}

impl<'de> Visitor<'de> for SplitVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "an array or object selected by split path '{}'",
            self.full_path
        )
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut batch = Vec::with_capacity(SPLIT_BATCH_SIZE);
        while let Some(key) = map.next_key::<String>()? {
            if self.cancelled.load(Ordering::Acquire) {
                return Err(A::Error::custom("split document consumer disconnected"));
            }
            let raw = map.next_value::<Box<RawValue>>().map_err(|error| {
                A::Error::custom(format!(
                    "object property '{key}' could not be deserialized: {error}"
                ))
            })?;
            batch.push(PendingDocument::Map { key, raw });
            send_full_batch(self.sender, &mut batch).map_err(A::Error::custom)?;
        }
        send_pending_batch(self.sender, batch).map_err(A::Error::custom)?;
        Ok(())
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut batch = Vec::with_capacity(SPLIT_BATCH_SIZE);
        let mut index = 0usize;
        loop {
            let raw = sequence.next_element::<Box<RawValue>>().map_err(|error| {
                A::Error::custom(format!(
                    "array element {index} could not be deserialized: {error}"
                ))
            })?;
            let Some(raw) = raw else {
                break;
            };
            if self.cancelled.load(Ordering::Acquire) {
                return Err(A::Error::custom("split document consumer disconnected"));
            }
            batch.push(PendingDocument::Array { index, raw });
            send_full_batch(self.sender, &mut batch).map_err(A::Error::custom)?;
            index += 1;
        }
        send_pending_batch(self.sender, batch).map_err(A::Error::custom)?;
        Ok(())
    }
}

fn send_full_batch(
    sender: &SyncSender<Vec<PendingDocument>>,
    batch: &mut Vec<PendingDocument>,
) -> std::result::Result<(), String> {
    if batch.len() < SPLIT_BATCH_SIZE {
        return Ok(());
    }
    let full = std::mem::replace(batch, Vec::with_capacity(SPLIT_BATCH_SIZE));
    send_pending_batch(sender, full)
}

fn send_pending_batch(
    sender: &SyncSender<Vec<PendingDocument>>,
    batch: Vec<PendingDocument>,
) -> std::result::Result<(), String> {
    if batch.is_empty() {
        return Ok(());
    }
    sender
        .send(batch)
        .map_err(|_| "split document consumer disconnected".to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        SPLIT_BATCH_SIZE, SplitEvent, SplitPath, start_split_reader,
        start_split_reader_with_worker_count,
    };
    use serde_json::Value;
    use std::{
        io::{Cursor, Read},
        sync::{
            Arc, Condvar, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    fn collect_raw(input: &str, path: &str) -> std::result::Result<Vec<String>, String> {
        let receiver = start_split_reader(
            Cursor::new(input.as_bytes().to_vec()),
            "fixture.json".to_string(),
            SplitPath::parse(path).unwrap(),
        )
        .unwrap();
        let mut documents = Vec::new();
        loop {
            match receiver.recv().unwrap() {
                SplitEvent::Documents(batch) => {
                    documents.extend(batch.into_iter().map(|raw| raw.get().to_string()));
                }
                SplitEvent::Failure(error) => return Err(error),
                SplitEvent::Complete => return Ok(documents),
            }
        }
    }

    fn collect(input: &str, path: &str) -> std::result::Result<Vec<Value>, String> {
        collect_raw(input, path).map(|documents| {
            documents
                .into_iter()
                .map(|raw| serde_json::from_str(&raw).unwrap())
                .collect()
        })
    }

    #[test]
    fn parses_root_and_normalized_paths() {
        assert_eq!(SplitPath::parse("").unwrap().tokens, Vec::<String>::new());
        assert_eq!(SplitPath::parse("/").unwrap().tokens, Vec::<String>::new());
        assert_eq!(SplitPath::parse("/hits").unwrap().tokens, vec!["hits"]);
        assert_eq!(SplitPath::parse("/hits/").unwrap().tokens, vec!["hits"]);
        assert_eq!(
            SplitPath::parse("/a~1b/m~0n").unwrap().tokens,
            vec!["a/b", "m~n"]
        );
    }

    #[test]
    fn rejects_invalid_paths() {
        assert!(
            SplitPath::parse("hits")
                .unwrap_err()
                .to_string()
                .contains("start")
        );
        assert!(SplitPath::parse("//").is_err());
        assert!(
            SplitPath::parse("/hits//")
                .unwrap_err()
                .to_string()
                .contains("final empty-name")
        );
        assert!(
            SplitPath::parse("/hits/~2")
                .unwrap_err()
                .to_string()
                .contains("~2")
        );
        assert!(
            SplitPath::parse("/hits/~")
                .unwrap_err()
                .to_string()
                .contains("incomplete")
        );
    }

    #[test]
    fn splits_root_map_with_string_ids() {
        let documents = collect(
            r#"{"20":{"name":"Beta"},"10":{"name":"Alpha","nested":[1,true]}}"#,
            "/",
        )
        .unwrap();
        assert_eq!(documents.len(), 2);
        let beta = documents.iter().find(|doc| doc["id"] == "20").unwrap();
        let alpha = documents.iter().find(|doc| doc["id"] == "10").unwrap();
        assert_eq!(beta["name"], "Beta");
        assert_eq!(alpha["nested"], serde_json::json!([1, true]));
    }

    #[test]
    fn splits_nested_array_without_synthetic_ids() {
        let documents = collect(
            r#"{"hits":[{"name":"Alpha"},{"name":"Beta","nested":{"ok":true}}]}"#,
            "/hits/",
        )
        .unwrap();
        assert_eq!(documents.len(), 2);
        assert!(documents.contains(&serde_json::json!({"name": "Alpha"})));
        assert!(documents.contains(&serde_json::json!({"name": "Beta", "nested": {"ok": true}})));
    }

    #[test]
    fn preserves_arbitrary_precision_numbers() {
        const LARGE_INTEGER: &str = "123456789012345678901234567890";
        const PRECISE_DECIMAL: &str = "0.123456789012345678901234567890";

        let array = collect_raw(
            &format!(r#"[{{"large":{LARGE_INTEGER},"decimal":{PRECISE_DECIMAL}}}]"#),
            "/",
        )
        .unwrap();
        assert_eq!(array.len(), 1);
        assert!(array[0].contains(LARGE_INTEGER));
        assert!(array[0].contains(PRECISE_DECIMAL));

        let map = collect_raw(
            &format!(r#"{{"730":{{"large":{LARGE_INTEGER},"decimal":{PRECISE_DECIMAL}}}}}"#),
            "/",
        )
        .unwrap();
        assert_eq!(map.len(), 1);
        assert!(map[0].contains(r#""id":"730""#));
        assert!(map[0].contains(LARGE_INTEGER));
        assert!(map[0].contains(PRECISE_DECIMAL));
    }

    #[test]
    fn traverses_escaped_keys_and_array_indices() {
        let documents = collect(
            r#"{"a/b":[{"skip":[]},{"m~n":{"x":{"value":1},"y":{"value":2}}}]}"#,
            "/a~1b/1/m~0n",
        )
        .unwrap();
        assert!(documents.contains(&serde_json::json!({"id": "x", "value": 1})));
        assert!(documents.contains(&serde_json::json!({"id": "y", "value": 2})));
    }

    #[test]
    fn accepts_empty_collections() {
        assert!(collect("{}", "/").unwrap().is_empty());
        assert!(collect("[]", "/").unwrap().is_empty());
    }

    #[test]
    fn reports_pointer_and_document_shape_errors() {
        let missing = collect(r#"{"hits":[]}"#, "/missing").unwrap_err();
        assert!(missing.contains("fixture.json"));
        assert!(missing.contains("missing"));

        let bad_index = collect(r#"{"hits":[[]]}"#, "/hits/01").unwrap_err();
        assert!(bad_index.contains("canonical zero-based index"));

        let missing_index = collect(r#"{"hits":[{}]}"#, "/hits/2").unwrap_err();
        assert!(missing_index.contains("array index '2'"));

        let scalar_traversal = collect(r#"{"hits":1}"#, "/hits/name").unwrap_err();
        assert!(scalar_traversal.contains("token 'name'"));
        assert!(scalar_traversal.contains("/hits/name"));

        let scalar = collect(r#"{"hits":1}"#, "/hits").unwrap_err();
        assert!(scalar.contains("array or object selected"));

        let map_child = collect(r#"{"bad":null}"#, "/").unwrap_err();
        assert!(map_child.contains("property 'bad'"));

        let array_child = collect(r#"[{} , 1]"#, "/").unwrap_err();
        assert!(array_child.contains("element 1"));

        let conflict = collect(r#"{"10":{"id":"existing"}}"#, "/").unwrap_err();
        assert!(conflict.contains("property '10'"));
        assert!(conflict.contains("generated 'id'"));
    }

    #[test]
    fn reports_malformed_and_trailing_json_after_prior_documents() {
        let malformed = collect(r#"[{"ok":true},{"bad":}]"#, "/").unwrap_err();
        assert!(malformed.contains("array element 1"));
        assert!(malformed.contains("line 1 column"));

        let malformed_map = collect(r#"{"good":{"ok":true},"bad":{"broken":}}"#, "/").unwrap_err();
        assert!(malformed_map.contains("object property 'bad'"));
        assert!(malformed_map.contains("line 1 column"));

        let trailing = collect(r#"[{"ok":true}] {}"#, "/").unwrap_err();
        assert!(trailing.contains("trailing characters"));
    }

    struct GatedReader {
        bytes: Cursor<Vec<u8>>,
        gate_at: usize,
        gate: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Read for GatedReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let position = self.bytes.position() as usize;
            if position >= self.gate_at {
                let (released, condition) = &*self.gate;
                let mut released = released.lock().unwrap();
                while !*released {
                    released = condition.wait(released).unwrap();
                }
                return self.bytes.read(buffer);
            }
            let allowed = buffer.len().min(self.gate_at - position);
            self.bytes.read(&mut buffer[..allowed])
        }
    }

    struct TrackingReader {
        bytes: Cursor<Vec<u8>>,
        bytes_read: Arc<AtomicUsize>,
    }

    impl Read for TrackingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let count = self.bytes.read(buffer)?;
            self.bytes_read
                .store(self.bytes.position() as usize, Ordering::Release);
            Ok(count)
        }
    }

    #[test]
    fn emits_before_reading_the_complete_collection() {
        let documents = (0..=SPLIT_BATCH_SIZE)
            .map(|index| format!(r#"{{"value":{index}}}"#))
            .collect::<Vec<_>>();
        let input = format!("[{}]", documents.join(","));
        let gate_at = input
            .match_indices(',')
            .nth(SPLIT_BATCH_SIZE - 1)
            .map(|(index, _)| index)
            .unwrap();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let receiver = start_split_reader(
            GatedReader {
                bytes: Cursor::new(input.into_bytes()),
                gate_at,
                gate: Arc::clone(&gate),
            },
            "gated.json".to_string(),
            SplitPath::parse("/").unwrap(),
        )
        .unwrap();
        match receiver.recv().unwrap() {
            SplitEvent::Documents(batch) => {
                assert_eq!(batch.len(), SPLIT_BATCH_SIZE);
                let values = batch
                    .into_iter()
                    .map(|raw| serde_json::from_str::<Value>(raw.get()).unwrap())
                    .collect::<Vec<_>>();
                assert!(values.iter().any(|value| value["value"] == 0));
            }
            SplitEvent::Failure(error) => panic!("unexpected failure: {error}"),
            SplitEvent::Complete => panic!("expected a document batch"),
        }
        let (released, condition) = &*gate;
        *released.lock().unwrap() = true;
        condition.notify_all();
        match receiver.recv().unwrap() {
            SplitEvent::Documents(batch) => {
                assert_eq!(batch.len(), 1);
                let value: Value = serde_json::from_str(batch[0].get()).unwrap();
                assert_eq!(value["value"], SPLIT_BATCH_SIZE);
            }
            SplitEvent::Failure(error) => panic!("unexpected failure: {error}"),
            SplitEvent::Complete => panic!("expected a second document batch"),
        }
        assert!(matches!(receiver.recv().unwrap(), SplitEvent::Complete));
    }

    #[test]
    fn bounded_handoff_stops_reading_when_consumer_stalls() {
        const DOCUMENTS: usize = 10_000;
        let padding = "x".repeat(128);
        let documents = (0..DOCUMENTS)
            .map(|index| format!(r#"{{"value":{index},"padding":"{padding}"}}"#))
            .collect::<Vec<_>>();
        let input = format!("[{}]", documents.join(","));
        let input_len = input.len();
        let bytes_read = Arc::new(AtomicUsize::new(0));
        let receiver = start_split_reader_with_worker_count(
            TrackingReader {
                bytes: Cursor::new(input.into_bytes()),
                bytes_read: Arc::clone(&bytes_read),
            },
            "backpressure.json".to_string(),
            SplitPath::parse("/").unwrap(),
            1,
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut previous = usize::MAX;
        let mut stable_samples = 0;
        while Instant::now() < deadline && stable_samples < 5 {
            std::thread::sleep(Duration::from_millis(10));
            let current = bytes_read.load(Ordering::Acquire);
            if current > 0 && current == previous {
                stable_samples += 1;
            } else {
                stable_samples = 0;
                previous = current;
            }
        }

        let stalled_at = bytes_read.load(Ordering::Acquire);
        assert_eq!(stable_samples, 5, "split parser did not reach backpressure");
        assert!(
            stalled_at < input_len,
            "split parser read the complete input while its consumer was stalled"
        );

        let mut emitted = 0;
        loop {
            match receiver.recv().unwrap() {
                SplitEvent::Documents(batch) => emitted += batch.len(),
                SplitEvent::Failure(error) => panic!("unexpected failure: {error}"),
                SplitEvent::Complete => break,
            }
        }
        assert_eq!(emitted, DOCUMENTS);
    }

    #[test]
    fn reports_late_error_after_an_emitted_batch() {
        let documents = (0..SPLIT_BATCH_SIZE)
            .map(|index| format!(r#"{{"value":{index}}}"#))
            .collect::<Vec<_>>();
        let input = format!(r#"[{},{{"bad":}}]"#, documents.join(","));
        let gate_at = input
            .match_indices(',')
            .nth(SPLIT_BATCH_SIZE - 1)
            .map(|(index, _)| index)
            .unwrap();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let receiver = start_split_reader(
            GatedReader {
                bytes: Cursor::new(input.into_bytes()),
                gate_at,
                gate: Arc::clone(&gate),
            },
            "late-error.json".to_string(),
            SplitPath::parse("/").unwrap(),
        )
        .unwrap();

        match receiver.recv().unwrap() {
            SplitEvent::Documents(batch) => assert_eq!(batch.len(), SPLIT_BATCH_SIZE),
            SplitEvent::Failure(error) => panic!("unexpected early failure: {error}"),
            SplitEvent::Complete => panic!("expected a document batch"),
        }

        let (released, condition) = &*gate;
        *released.lock().unwrap() = true;
        condition.notify_all();
        match receiver.recv().unwrap() {
            SplitEvent::Failure(error) => {
                assert!(error.contains("late-error.json"));
                assert!(error.contains("line 1 column"));
                assert!(!error.contains("roll"));
            }
            SplitEvent::Documents(_) => panic!("unexpected document batch after malformed JSON"),
            SplitEvent::Complete => panic!("expected malformed JSON failure"),
        }
    }

    #[test]
    fn splits_collections_larger_than_the_default_bulk_batch() {
        const DOCUMENTS: usize = 5_001;

        let mut map = String::from("{");
        let mut array = String::from("[");
        for index in 0..DOCUMENTS {
            if index > 0 {
                map.push(',');
                array.push(',');
            }
            map.push_str(&format!(r#""{index}":{{"value":{index}}}"#));
            array.push_str(&format!(r#"{{"value":{index}}}"#));
        }
        map.push('}');
        array.push(']');

        let mut map_documents = collect(&map, "/").unwrap();
        let mut array_documents = collect(&array, "/").unwrap();
        assert_eq!(map_documents.len(), DOCUMENTS);
        assert_eq!(array_documents.len(), DOCUMENTS);
        map_documents
            .sort_by_key(|document| document["id"].as_str().unwrap().parse::<usize>().unwrap());
        array_documents.sort_by_key(|document| document["value"].as_u64().unwrap());
        for index in 0..DOCUMENTS {
            assert_eq!(map_documents[index]["id"], index.to_string());
            assert_eq!(map_documents[index]["value"], index);
            assert_eq!(array_documents[index]["value"], index);
        }
    }
}
