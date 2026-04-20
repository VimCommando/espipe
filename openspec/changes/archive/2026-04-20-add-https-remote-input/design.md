## Context

`espipe` currently builds an `Input` from a URI-like argument and supports local `.ndjson` and `.csv` files plus `stdin`. `http://` and `https://` inputs are recognized syntactically, but `Input::read_line` returns an unimplemented error for URL sources. The ingest loop itself is intentionally simple and synchronous at the input boundary: it repeatedly calls `read_line`, receives a `Box<RawValue>`, and forwards documents to the configured output.

The requested change is intentionally narrow. We only need to support public HTTPS resources, with no authentication, redirects policy changes, or advanced transport settings. The implementation should preserve the current input parsing behavior instead of introducing a new streaming model or a parallel ingestion path.

## Goals / Non-Goals

**Goals:**
- Allow `https://.../*.csv`, `https://.../*.ndjson`, and `https://.../*.json` inputs.
- Fetch the remote body once at startup with `reqwest`.
- Reuse the existing CSV and NDJSON parsing logic after the download completes.
- Allow remote format detection when the URL path does not include an explicit supported extension.
- Keep remote input support out of the Elasticsearch output path and auth handling.

**Non-Goals:**
- Support authenticated remote inputs.
- Support plain `http://` remote input.
- Stream remote bodies incrementally during ingest.
- Add support for arbitrary JSON arrays or other non-line-delimited JSON formats.

## Decisions

### Use a startup-time HTTPS fetch into a temp file

Remote input should be fetched before the main ingest loop begins, written to a temp file with the matching extension, and then handed off to the existing file-based input path. This keeps the core ingestion loop unchanged and avoids mixing async network reads into the document-by-document parse path.

Why this over streaming:
- It is much simpler to integrate with the current synchronous `Input::read_line` interface.
- The requested scope explicitly favors a simple implementation.
- It keeps remote-source failure modes at startup instead of mid-stream.
- It reuses the existing file-backed CSV and NDJSON readers with minimal branching in the input layer.

Trade-off:
- The implementation must manage temp-file lifecycle correctly through successful completion and early failure paths.

### Restrict remote input to `https://`

Only HTTPS URLs should be accepted for remote input. Local file support already covers development workflows, and the requested behavior explicitly calls for HTTPS retrieval.

Why this over accepting both HTTP and HTTPS:
- It matches the requested scope.
- It avoids introducing a plaintext transport mode that would need separate documentation and review.

### Detect format from URL path first, then HTTP metadata

Remote `.csv`, `.ndjson`, and `.json` files should map onto the existing input modes using the URL path suffix when present. When the URL path does not include a supported extension, the client should send explicit `Accept` headers for CSV and NDJSON-oriented JSON responses and then infer the input mode from the response `Content-Type`.

Why this over extension-only detection:
- It supports stable download URLs that do not expose a file extension.
- It still preserves the current extension-based behavior as the simplest and most predictable path.

Why this over body sniffing:
- It matches the current local-file behavior, which already keys off file extension.
- It avoids guessing format from payload contents beyond the parser’s normal validation behavior.

### Treat remote `.json` the same as remote `.ndjson`

The first implementation should treat `.json` as line-delimited JSON input, just like `.ndjson`. This keeps the parser model aligned with the existing ingest loop, which expects one JSON object per logical line. If a fetched `.json` payload does not conform to the required NDJSON shape, startup should fail gracefully with the message `JSON payload does not look like required NDJSON input format.`

Why this over supporting arbitrary JSON documents:
- The current input pipeline is line-oriented.
- Supporting arrays or pretty-printed JSON objects would require a separate parser shape and a broader design change.

## Risks / Trade-offs

- [Downloaded temp files could be left behind on failure] → Tie temp-file ownership to the input object so cleanup happens when the process exits the ingest path.
- [Users may expect plain `.json` to support arrays or pretty-printed documents] → Fail with `JSON payload does not look like required NDJSON input format.` when JSON content is not valid NDJSON and document that `.json` is NDJSON-only in this change.
- [Remote URLs without extensions may advertise ambiguous content types] → Prefer explicit extension matching first and fail startup when the response `Content-Type` cannot be mapped to CSV or NDJSON input.
- [Remote fetch failures happen before any document processing begins] → Surface explicit startup errors for non-success responses, unsupported schemes, and network failures.
- [Adding a direct HTTP client dependency increases binary surface area] → Keep the dependency scope narrow and use only the minimal GET flow needed for this feature.

## Migration Plan

No migration is required. This change adds a new input mode without changing existing local file, stdin, or Elasticsearch output behavior.

## Open Questions

- None.
