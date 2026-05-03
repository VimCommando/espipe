## Context

`espipe` currently detects input behavior from file suffixes and routes local `.csv`, `.ndjson`, and `.json` files through `src/input.rs`. CSV readers already consume `Box<dyn Read + Send>`, and NDJSON readers consume a `BufReader<Box<dyn Read + Send>>`, so gzip decoding can be introduced as another reader layer before the existing parsers. File output is centralized in `src/output/file.rs` and writes raw JSON bytes plus a newline for each document.

Elasticsearch request-body gzip already exists as a separate concern behind the `--uncompressed` option. This change is about gzip file encoding and decoding only.

## Goals / Non-Goals

**Goals:**
- Read local `.csv.gz` inputs as CSV after gzip decompression.
- Read local `.ndjson.gz` inputs as NDJSON after gzip decompression.
- Write gzip-compressed NDJSON when the selected file output path ends in `.ndjson.gz`.
- Preserve the current raw document pipeline and uncompressed file behavior.
- Keep gzip handling streaming so large files do not require full decompression into memory or a temporary file.

**Non-Goals:**
- Support arbitrary compressed input formats beyond gzip.
- Support gzip-compressed `.json.gz`, file-document imports, stdin, stdout, or remote HTTPS inputs in this change.
- Change Elasticsearch bulk request compression behavior or the meaning of `--uncompressed`.
- Add CLI flags for compression selection; suffix-based behavior is sufficient for this proposal.

## Decisions

### Detect supported compound suffixes before simple extensions

Input and output path classification should recognize `.csv.gz` and `.ndjson.gz` as compound suffixes before consulting `Path::extension()`. This avoids treating every `.gz` file as an unknown generic gzip file and keeps behavior explicit.

Alternatives considered:
- Inspect gzip contents or infer the inner format from decompressed data. This was rejected because `espipe` already uses suffix-based local file behavior, and inference would add ambiguity and startup cost.
- Accept all `*.gz` inputs. This was rejected because unsupported gzip payloads would fail later with parser-specific errors rather than clear format validation.

### Add gzip codecs at the file I/O boundary

Local compressed inputs should open the file, wrap it in `flate2::read::GzDecoder`, and pass that reader to the existing CSV or NDJSON reader construction. Compressed file output should create the destination file, wrap it in `flate2::write::GzEncoder`, and reuse the existing raw NDJSON write behavior.

Alternatives considered:
- Decompress to a temporary file and reuse only `File` readers. This was rejected because it increases disk usage and delays ingestion for large files.
- Add gzip awareness inside CSV or NDJSON parsing functions. This was rejected because compression is an I/O concern, and the parser code already works against generic readers.

### Generalize file output writer ownership

`FileOutput` currently stores `BufWriter<File>`. To support gzip output, it should store a buffered writer over a `Box<dyn Write + Send>` or equivalent enum so both plain files and gzip encoders share the same `Sender` implementation. Closing output must finish the gzip stream before reporting success.

Alternatives considered:
- Add a separate `GzipFileOutput` output variant. This was rejected because the send semantics are identical and a single `FileOutput` can hide the writer details.

### Keep remote gzip out of scope

This change should not add `https://.../*.ndjson.gz` or content-encoding handling. Remote input currently stages fetched content to a temp file and infers supported formats from URL suffix or content type. Adding compressed remote input involves separate decisions around `Content-Encoding`, `Content-Type`, temp-file suffixes, and validation.

Alternatives considered:
- Include remote gzip now. This was rejected to keep the first implementation focused on local file compression requested by the proposal.

## Risks / Trade-offs

- [Gzip writer is not finalized before process exit] -> Ensure `Output::close` consumes the output and calls the encoder finish path, and cover this with a test that reads the resulting gzip file.
- [Compound suffix detection regresses existing `.json`, `.jsonl`, or file-document behavior] -> Add targeted tests for `.csv`, `.ndjson`, `.csv.gz`, `.ndjson.gz`, and unsupported `.json.gz`.
- [Multi-file import output validation rejects `.ndjson.gz`] -> Update the multi-input file-output guard so compressed NDJSON output is accepted wherever uncompressed `.ndjson` output is accepted.
- [New dependency increases compile surface] -> Use `flate2`, which provides maintained streaming `Read` and `Write` gzip adapters and fits the existing synchronous file I/O model.

## Migration Plan

No data migration is required. Existing uncompressed inputs and outputs continue to behave the same. Rollback is removing the gzip suffix handling and dependency; compressed files created during the feature window remain standard gzip files that users can decompress externally.

## Open Questions

- None.
