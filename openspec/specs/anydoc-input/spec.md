## Purpose

Define conversion of supported local non-text documents through the Rust `anydoc` processor while preserving espipe's existing file-document behavior.

## Requirements

### Requirement: Supported local non-text files are converted through anydoc

The system SHALL use the Rust `anydoc` processor for local regular files with these extensions: `.doc`, `.docx`, `.docm`, `.odt`, `.pdf`, `.ppt`, `.pps`, `.pot`, `.pptx`, `.pptm`, `.ppsx`, `.ppsm`, `.rtf`, `.epub`, `.xls`, `.xlsx`, `.xlsm`, `.xlsb`, `.ods`, and `.odp`. The processor SHALL convert each file to GitHub-Flavored Markdown before file-document construction.

#### Scenario: A PDF file is imported

- **WHEN** the user runs `espipe` with a local text-based `.pdf` input
- **THEN** the system converts the PDF to Markdown through anydoc
- **AND** emits one JSON document containing the converted Markdown in the configured content field

#### Scenario: An office document is imported

- **WHEN** the user runs `espipe` with a local supported Word, PowerPoint, Excel, or OpenDocument input
- **THEN** the system converts the file to Markdown through anydoc
- **AND** emits one JSON document for the source file

#### Scenario: A supported file is imported from a mixed local collection

- **WHEN** file-document input resolves supported anydoc files together with Markdown or text files
- **THEN** each anydoc file is converted at its position in the deterministic file order
- **AND** existing Markdown and text files continue through their existing readers

### Requirement: Anydoc conversion preserves the existing file-document shape

The system SHALL construct converted documents using the existing file-document semantics after Markdown conversion. It SHALL store converted Markdown under `content.<field_name>`, apply the configured `--content` value, and add one `origin` metadata object for multi-file or glob-resolved local imports. The object SHALL contain `scheme`, `authority`, `path`, `query`, `fragment`, and `filename` fields. It SHALL NOT emit the legacy `file.path` or `file.name` metadata.

#### Scenario: Default content field is used for converted Markdown

- **WHEN** a supported anydoc file is imported without `--content`
- **THEN** the converted Markdown is stored in `content.body`
- **AND** the document does not expose the original binary bytes as a field

#### Scenario: Custom content field is used for converted Markdown

- **WHEN** a supported anydoc file is imported with `--content markdown`
- **THEN** the converted Markdown is stored in `content.markdown`
- **AND** the system does not add `content.body` solely because the source was converted

#### Scenario: Converted files participate in multi-file origin metadata

- **WHEN** anydoc files are imported together with another file-document input
- **THEN** each converted document includes an `origin` object
- **AND** its URI components identify the original local file

#### Scenario: Glob-resolved converted files include complete origin metadata

- **WHEN** anydoc files are imported through a local glob pattern
- **THEN** each converted document includes `origin.scheme` equal to `file`
- **AND** `origin.path` identifies the containing directory
- **AND** `origin.filename` identifies the original local file
- **AND** absent `origin.authority`, `origin.query`, and `origin.fragment` fields are omitted

#### Scenario: Remote inputs preserve URI origin metadata

- **WHEN** an HTTP or HTTPS CSV, NDJSON, or Toon input is imported
- **THEN** each emitted document includes `origin.scheme`, `origin.authority`, `origin.path`, `origin.query`, `origin.fragment`, and `origin.filename`
- **AND** the values reflect the source URI rather than the temporary download file

### Requirement: Existing local discovery mechanisms support anydoc files

The system SHALL process supported anydoc files supplied as direct local paths, shell-expanded file lists, or existing local glob patterns. It SHALL not require a separate subprocess or remote service for local conversion.

#### Scenario: A quoted recursive PDF glob is imported

- **WHEN** the user runs `espipe '**/*.pdf' output.ndjson`
- **THEN** the existing glob resolver finds matching regular files
- **AND** each matching PDF is converted and emitted as one file document

#### Scenario: Multiple extension patterns are supplied

- **WHEN** the user supplies separate local input patterns such as `**/*.pdf`, `**/*.xls`, and `**/*.doc`
- **THEN** the system combines and de-duplicates the resolved paths using existing file discovery rules
- **AND** converts each supported path according to its extension

### Requirement: Anydoc conversion failures identify the source file

The system SHALL report anydoc conversion failures through the existing file-input error path, including the source path and the underlying conversion reason when available. It SHALL not emit a synthetic document for a file that anydoc cannot convert.

#### Scenario: An unsupported document is encountered

- **WHEN** anydoc reports that a supported-extension file is encrypted, malformed, unsupported, or exceeds a conversion limit
- **THEN** ingestion fails with a diagnostic identifying the source path
- **AND** the diagnostic is written to stderr

#### Scenario: An image-only PDF is encountered

- **WHEN** anydoc cannot extract meaningful text from a scanned or image-only PDF
- **THEN** ingestion fails with a path-specific unsupported-conversion diagnostic
- **AND** the system does not claim to perform OCR
