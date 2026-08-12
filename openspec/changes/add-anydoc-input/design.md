## Context

Local file imports already resolve concrete paths and recursive glob patterns in `src/input.rs`. The `Input::FileDocuments` variant reads each path lazily, constructs a JSON object, and sends it through the same `Box<RawValue>` output path used by NDJSON, CSV, and other inputs. Today, recognized text formats have specialized readers and all other file-document paths fall back to UTF-8 text, which rejects PDFs and office containers.

The `anydoc` crate provides a Rust-native conversion API for PDF, Word, PowerPoint, Excel, OpenDocument, RTF, and EPUB inputs. Its Markdown output is the appropriate intermediate representation because the existing file-document implementation already defines the desired content field, Markdown frontmatter behavior, and multi-file metadata.

## Goals / Non-Goals

**Goals:**

- Convert supported local anydoc formats into the existing file-document JSON shape.
- Preserve the existing output dispatch and behavior for Markdown, text, YAML, JSON, NDJSON, Toon, CSV, stdin, and HTTPS inputs.
- Support direct files, shell-expanded file lists, and existing recursive glob patterns without changing discovery semantics.
- Keep conversion errors path-specific and visible through the existing stderr error path.
- Cover representative document formats and mixed file collections with tests.

**Non-Goals:**

- OCR for scanned or image-only PDFs.
- Remote HTTPS anydoc inputs.
- A new `--extensions` option; multiple extension patterns can be passed as existing input positionals.
- Extraction of embedded assets or source-specific metadata beyond the Markdown produced by anydoc and existing `file.*` metadata.
- Content-based conversion of unknown-extension files, which could change the existing unknown UTF-8 text behavior.

## Decisions

### Use the `anydoc` crate directly

Add `anydoc` as a normal dependency and call its Rust API rather than spawning the anydoc CLI. This avoids subprocess lifecycle and temporary-file concerns and keeps conversion inside the existing input processor. The dependency is compatible with the repository's Rust 1.88 baseline.

Alternatives considered:

- **Invoke the anydoc CLI:** rejected because it adds an external executable/runtime dependency and makes error handling and streaming less direct.
- **Add format-specific parsers to espipe:** rejected because it duplicates the purpose of anydoc and expands the maintenance surface.

### Gate conversion by recognized non-CSV extension

In `read_file_documents`, check `anydoc::Format::from_path(path)` after the existing specialized readers. If it identifies a supported format other than `Format::Csv`, convert the file with `anydoc::to_markdown(path)`. Keep existing readers ahead of this branch so CSV and all current text/document formats retain their behavior.

The extension gate intentionally avoids running content detection on every unknown file. Unknown valid UTF-8 files remain text documents, while recognized anydoc extensions opt into conversion. Mislabeled-format support can be considered separately if needed.

### Reuse the Markdown document builder

Refactor the existing Markdown reader so it has a helper that accepts `(path, markdown_text, content_field, include_file_metadata)`. The current Markdown path reads the source text and calls this helper; the anydoc path converts the source and calls the same helper.

This preserves:

- `content.<field>` placement and `--content` behavior.
- YAML frontmatter extraction and content-field conflict checks.
- `file.path` and `file.name` inclusion for multi-file imports, plus the containing directory in `origin.path` and the basename in `origin.filename` for glob-resolved imports.
- One serialized `Box<RawValue>` per output document.

The resulting flow is:

```text
path/glob → existing path resolution → format-specific input branch
                                      ├─ Markdown → Markdown document builder
                                      ├─ anydoc format → anydoc Markdown → same builder
                                      └─ existing text/JSON/YAML/etc. readers
              → Box<RawValue> → existing output
```

### Keep discovery unchanged

No new traversal or extension-selection option is needed. Existing commands such as `espipe '**/*.pdf' output.ndjson` work because unknown local extensions already enter file-document mode, and multiple types can be supplied as separate patterns. A future `--extensions pdf,xls,doc` option should be designed as a discovery/filtering feature rather than coupled to the converter.

### Preserve lazy conversion

Convert files when `read_file_document_line` reaches them, matching the current lazy file-document behavior. This avoids loading an entire collection before output begins and keeps anydoc conversion scoped to the selected file. Conversion failures use the existing input error path and include the source path.

### Preserve anydoc error wording

Use anydoc's conversion error wording as-is, adding only the source path context needed to identify the failing input. This avoids introducing a second error taxonomy while preserving the underlying reason for failures. Error normalization can be added later if callers need a stable espipe-specific code or prefix.

## Risks / Trade-offs

- **Dependency and binary growth** → Accept the direct dependency for broad format coverage; document the build impact and verify the normal test/build path.
- **Full-file memory use** → Keep the existing one-file-at-a-time document model; anydoc already returns a Markdown `String`, and the resulting raw document is released before the next file is read.
- **Scanned PDFs fail without OCR** → Surface anydoc's unsupported conversion error with the source path; do not claim OCR support.
- **Failures can occur after earlier documents were sent** → Retain existing lazy input semantics and error reporting; changing ingestion to preflight every file is outside this change.
- **Generated Markdown may contain frontmatter-like syntax** → Run it through the established Markdown builder for consistent document semantics; add a fixture-based regression test for representative anydoc output.
- **Unknown-extension detection remains extension-gated** → Preserve current unknown UTF-8 behavior; consider content-based detection only as a separate, explicitly scoped change.

## Migration Plan

No data migration is required. Add the dependency and input branch, then release normally. Existing commands and output document shapes remain compatible. Rollback consists of removing the anydoc dependency, branch, tests, and change artifacts; no stored documents require migration.

## Open Questions

None. The initial regression suite will use one representative PDF and one Office/OpenDocument fixture; coverage of every supported format remains the responsibility of the anydoc crate.
