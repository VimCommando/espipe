## MODIFIED Requirements

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
- **THEN** each anydoc file is converted by the bounded worker pool
- **AND** documents may be emitted in conversion completion order
- **AND** existing Markdown and text files continue through their existing readers

## ADDED Requirements

### Requirement: Multi-source document conversion uses bounded concurrency

The system SHALL convert files from a multi-source local document import concurrently. It SHALL bound the number of active conversions and completed conversion results retained in memory. Concurrent conversion SHALL emit source results as workers complete while preserving generated document identity and per-file error recovery behavior.

#### Scenario: Multiple PDFs are converted concurrently

- **WHEN** a multi-source local import contains more convertible PDFs than the conversion worker limit
- **THEN** the system permits multiple PDF conversions to execute at the same time
- **AND** it does not start an unbounded number of conversion operations

#### Scenario: Concurrent conversions finish in a different order from discovery

- **WHEN** a later file finishes conversion before an earlier file
- **THEN** the system emits the completed result without waiting for the earlier file
- **AND** output order is not guaranteed to match source-path order

#### Scenario: A concurrent conversion fails

- **WHEN** one file fails to read or convert while other file conversions are active
- **THEN** the system logs the same path-specific warning used by batch file recovery
- **AND** emits no document for the failed file
- **AND** continues emitting successful documents as conversions finish

#### Scenario: Generated IDs are produced by concurrent conversion

- **WHEN** multi-source conversion completes files in a different order across two runs
- **THEN** each emitted document receives the same generated ID in both runs
