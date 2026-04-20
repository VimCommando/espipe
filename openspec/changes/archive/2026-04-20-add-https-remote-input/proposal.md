## Why

`espipe` already accepts URI-like input arguments, but `http://` and `https://` inputs are currently parsed and then rejected at runtime. That leaves a gap for simple ingestion workflows where the source data already lives at a public HTTPS URL and does not need any authentication or pre-download step.

## What Changes

- Add HTTPS remote input support for unauthenticated source files.
- Fetch remote input bodies with a simple `reqwest` GET before ingestion begins.
- Support remote `.csv`, `.ndjson`, and `.json` inputs over `https://`.
- Treat remote `.ndjson` and `.json` sources as newline-delimited JSON input in the existing ingest pipeline and fail gracefully when a `.json` payload is not valid NDJSON.
- Allow remote input format detection to fall back to HTTP metadata when the URL path does not end in an explicit supported extension.
- Return a startup error for unsupported remote input cases such as non-HTTPS URLs, unrecognized remote content types, HTTP error responses, or network failures.

## Capabilities

### New Capabilities
- `https-remote-input`: Read unauthenticated remote input files over HTTPS and feed them into the existing CSV or JSON ingest pipeline.

### Modified Capabilities
- `rawvalue-document-pipeline`: Extend input-source requirements so the raw-value ingest path accepts fetched HTTPS JSON sources in addition to local files and stdin.

## Impact

- Affected code: `src/input.rs`, `src/main.rs`, and any call sites that construct inputs.
- Dependencies: add a direct `reqwest` dependency for HTTPS fetches.
- Behavior: startup now performs a remote fetch for supported HTTPS input URIs, stages the content in a temp file, and then begins ingestion through the existing file-based path.
