## 1. Dependency and input integration

- [ ] 1.1 Add the `anydoc` crate dependency at a Rust 1.88-compatible version.
- [ ] 1.2 Add extension-gated anydoc routing to `read_file_documents`, preserving the existing specialized readers and excluding CSV from the new branch.
- [ ] 1.3 Refactor Markdown file-document construction to accept converted Markdown text so source Markdown and anydoc output share the same content-field, frontmatter, and file-metadata behavior.
- [ ] 1.4 Add the anydoc conversion wrapper using `anydoc::to_markdown`, including source-path context and underlying conversion errors in diagnostics.
- [ ] 1.5 Verify direct paths, shell-expanded file lists, recursive globs, mixed anydoc/Markdown collections, deterministic ordering, and lazy conversion continue to use the existing discovery and output pipeline.

## 2. AnyDoc format coverage and error behavior

- [ ] 2.1 Add representative routing and conversion coverage for one text-based PDF and one Office or OpenDocument file; rely on anydoc's own test suite for coverage of its remaining formats.
- [ ] 2.2 Verify the anydoc routing is driven by the crate's recognized format API and excludes existing CSV handling.
- [ ] 2.3 Add tests for unsupported, malformed, encrypted, or image-only conversion failures and verify diagnostics identify the source path and are written to stderr.
- [ ] 2.4 Add a regression test proving unknown valid UTF-8 files and existing Markdown, YAML, JSON, NDJSON, Toon, CSV, stdin, and HTTPS inputs retain their current behavior.

## 3. Output-shape and integration verification

- [ ] 3.1 Verify converted documents use `content.body` by default and the configured `--content` field when specified.
- [ ] 3.2 Verify converted documents preserve existing `file.path` and `file.name` behavior for multi-file imports and omit it for a single direct file.
- [ ] 3.3 Verify mixed recursive file imports emit converted and existing documents in the same deterministic path order without changing output serialization.
- [ ] 3.4 Update README input documentation with supported anydoc formats, local-only behavior, glob examples, and the limitation that scanned PDFs require OCR outside espipe.

## 4. Verification

- [ ] 4.1 Run formatting and the complete Rust test suite.
- [ ] 4.2 Run the CLI/integration tests that write NDJSON output and confirm existing output consumers require no changes.
