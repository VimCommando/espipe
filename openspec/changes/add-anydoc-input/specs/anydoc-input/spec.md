## ADDED Requirements

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

The system SHALL construct converted documents using the existing file-document semantics after Markdown conversion. It SHALL store converted Markdown under `content.<field_name>`, apply the configured `--content` value, and add `file.path` and `file.name` only under the existing multi-file rules.

#### Scenario: Default content field is used for converted Markdown

- **WHEN** a supported anydoc file is imported without `--content`
- **THEN** the converted Markdown is stored in `content.body`
- **AND** the document does not expose the original binary bytes as a field

#### Scenario: Custom content field is used for converted Markdown

- **WHEN** a supported anydoc file is imported with `--content markdown`
- **THEN** the converted Markdown is stored in `content.markdown`
- **AND** the system does not add `content.body` solely because the source was converted

#### Scenario: Converted files participate in multi-file metadata

- **WHEN** anydoc files are imported together with another file-document input
- **THEN** each converted document includes the same `file.path` and `file.name` metadata as existing file documents
- **AND** the metadata values identify the original local file

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
