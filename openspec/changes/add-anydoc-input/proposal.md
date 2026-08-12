## Why

`espipe` currently treats local file documents as text, so PDFs and office formats cannot be ingested even though the existing glob-based file import pipeline already supports recursive document collections. The Rust `anydoc` library can convert these formats to GitHub-Flavored Markdown, allowing non-text ingestion without changing the downstream document or output model.

## What Changes

- Add `anydoc` as a local file input preprocessor for supported non-text formats, including PDF, Word, PowerPoint, Excel, OpenDocument, RTF, and EPUB files.
- Convert each supported file to Markdown before constructing the existing file document, preserving `content.<field>`, Markdown handling, and conditional `file.*` metadata.
- Allow existing concrete file lists and recursive glob patterns such as `**/*.pdf` to ingest converted documents.
- Preserve existing CSV, JSON, NDJSON, Toon, Markdown, YAML, text, stdin, and HTTPS behavior.
- Report conversion failures with the source path and keep remote HTTPS inputs out of scope.
- Do not add `--extensions` in this change; multiple extension patterns can already be supplied as separate local inputs. A future extension-filter option can build on the existing discovery layer.

## Capabilities

### New Capabilities

- `anydoc-input`: Convert supported local non-text documents to Markdown within the input pipeline.

### Modified Capabilities

- `file-document-import`: Supported anydoc formats are imported as converted Markdown documents instead of being rejected as binary files.

## Impact

- Affects `src/input.rs` only in the ingestion path and adds the `anydoc` crate dependency.
- Adds fixtures and unit/integration coverage for representative converted formats, mixed globs, metadata preservation, and conversion errors.
- Increases dependency graph, compile time, and binary size because anydoc includes parsers for office containers and PDFs.
- Image-only/scanned PDFs remain unsupported because conversion does not provide OCR.
