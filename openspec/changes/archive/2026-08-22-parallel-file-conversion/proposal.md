## Why

Multi-source document imports convert files serially, so large PDF collections leave most CPU cores idle and delay the first Elasticsearch bulk request. The default 5,000-document bulk size compounds the delay for one-document-per-file imports.

## What Changes

- Convert multi-source file documents with a bounded worker pool sized for the local machine.
- Emit multi-source file documents as conversion workers finish while preserving generated IDs, bounded memory use, and per-file warn-and-skip behavior.
- Default Elasticsearch bulk requests to 500 documents for multi-source local input and retain 5,000 for single-file streaming input.
- Keep an explicit `--batch-size` value authoritative in every input mode.
- Change file-import completion summaries to report documents before source-file counts.
- Add regression tests for concurrent conversion, completion-order results, skipped files, and source-aware bulk defaults.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `anydoc-input`: Multi-source local document conversion becomes bounded and concurrent, with results emitted in completion order without changing identity or error recovery.
- `file-document-import`: Multi-source file output order becomes unspecified while deterministic discovery and de-duplication remain intact.
- `rawvalue-document-pipeline`: The default Elasticsearch bulk batch size depends on whether input is a multi-source local import or a single-file stream.

## Impact

The change affects local input construction and iteration in `src/input.rs`, completion summaries and Elasticsearch configuration selection in `src/main.rs`, related integration and unit tests, CLI documentation, and performance notes. It does not add a dependency or change explicit CLI option behavior.
