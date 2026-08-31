---
type: Reference
title: Input
description: Supported sources, document conversion, JSON splitting, and local file discovery rules.
resource: https://github.com/VimCommando/espipe/blob/main/docs/input.md
tags:
  - espipe
  - input
  - ingestion
  - files
status: stable
---

# Input

Each command accepts one or more inputs followed by one [output](output.md).

## Supported input forms

- `-`
  Reads NDJSON from `stdin`.
- `path/to/file.ndjson`, `path/to/file.json`, or `path/to/file.csv`
  Reads a supported local data file. Add `.gz` for compressed NDJSON or CSV.
- `path/to/file.pdf` or `path/to/file.docx`
  Converts a supported local document to Markdown.
- `'docs/**/*.pdf'`
  Recursively finds local PDFs and converts each one to a file document.
- `path/to/file.pdf path/to/file.xlsx output.ndjson`
  Imports multiple local file inputs and emits each source as its conversion finishes.

HTTP and HTTPS inputs support unauthenticated remote CSV, NDJSON, JSON, and Toon. If the URL has no recognized extension, `espipe` uses its `Content-Type`.

## AnyDoc local documents

Local files with these extensions are converted to GitHub-Flavored Markdown through anydoc before entering the existing file-document pipeline:

`.doc`, `.docx`, `.docm`, `.odt`, `.pdf`, `.ppt`, `.pps`, `.pot`, `.pptx`, `.pptm`, `.ppsx`, `.ppsm`, `.rtf`, `.epub`, `.xls`, `.xlsx`, `.xlsm`, `.xlsb`, `.ods`, and `.odp`.

Converted Markdown is stored in `content.body`; use `--content markdown` to change the field. Anydoc conversion is local only. Scanned PDFs require external OCR.

Multi-file and glob imports log per-file read or conversion failures and continue. Conversion uses up to eight workers and emits each source when it finishes, so cross-file order is unspecified. Generated IDs remain stable across runs, allowing for upsert operations.

## Data format rules

### NDJSON input

Each line must be valid line-delimited JSON. For pass-through JSON inputs, `espipe` expects the first non-whitespace character on each line to be `{`.

### Split JSON input

Use `--split /` for a root array or object, or a JSON Pointer such as `--split /hits` for a nested collection. One trailing slash is ignored. Pointer tokens use `~1` for `/` and `~0` for `~`; numeric tokens traverse arrays by zero-based index.

Each selected array element becomes one document. Each selected object value becomes one document with its key added as a string `id`. Existing `id` fields, non-object children, missing paths, and scalar or null selections are errors.

Split parsing is incremental and parallel. It does not preserve source order, and a late parse error does not roll back documents already sent.

### CSV input

The first row must be a header row. Each subsequent row is converted into a JSON object using the CSV headers as field names.

CSV values are emitted as JSON strings. `espipe` does not infer numeric, boolean, or date types from CSV input.

### Local file inputs

Markdown, text, YAML, JSON, NDJSON, JSONL, CSV, Toon, and anydoc-converted files become JSON documents. Markdown frontmatter is stored under `content.*`. Duplicate keys warn and use the last value; other invalid frontmatter is fatal.

Every local file document includes `origin.scheme: "file"`, a working-directory-relative `origin.path`, and `origin.filename`. Multi-source discovery skips symlinks and hidden paths by default. Use `--symlinks=follow|fail` and `--hidden=include|fail` to change those policies. Direct single-file input is not subject to discovery filtering.

Multi-source local inputs get deterministic IDs by default. Use `--generate-id=true` for a single source or `--generate-id=false` to disable them. IDs depend on the bundle, relative source path, and document position, not file contents or timestamps.
