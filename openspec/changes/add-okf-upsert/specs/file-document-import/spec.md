## MODIFIED Requirements

### Requirement: Multi-file imports include file metadata

The system SHALL add generalized `origin` metadata to every emitted local file document. For local file inputs, `origin.scheme` SHALL be `file`, `origin.path` SHALL be the directory path relative to the working directory, and `origin.filename` SHALL be the final file name. A file at the working-directory root SHALL use `./` as its relative origin path.

#### Scenario: A local file document includes origin metadata

- **WHEN** the user imports a local Markdown, text, YAML, JSON, NDJSON, JSONL, Toon, or converted document file
- **THEN** the emitted document includes an `origin` object
- **AND** `origin.scheme` is `file`
- **AND** `origin.path` is relative to the working directory
- **AND** `origin.filename` is the file's final path component

#### Scenario: Multiple files are imported

- **WHEN** file-document input resolves to more than one regular file
- **THEN** each emitted document includes `origin.scheme` equal to `file`
- **AND** each emitted document includes its relative `origin.path`
- **AND** each emitted document includes its `origin.filename`

#### Scenario: Single direct file is imported

- **WHEN** file-document input resolves to one direct file without a glob
- **THEN** the emitted document includes `origin.scheme` equal to `file`
- **AND** the emitted document includes a relative `origin.path`
- **AND** the emitted document includes its `origin.filename`

#### Scenario: A nested file preserves its relative directory

- **WHEN** the working directory contains `guides/getting-started.md`
- **AND** the user imports that file
- **THEN** the emitted document contains `origin.path` equal to `guides`
- **AND** the emitted document contains `origin.filename` equal to `getting-started.md`

#### Scenario: A root-level file uses the root origin path

- **WHEN** the working directory contains `README.md`
- **AND** the user imports that file
- **THEN** the emitted document contains `origin.path` equal to `./`
- **AND** the emitted document contains `origin.filename` equal to `README.md`

#### Scenario: Origin metadata does not expose the checkout absolute path

- **WHEN** the same bundle is imported from two checkout locations
- **THEN** the emitted local-file origins use the same relative path and filename
- **AND** neither origin contains the checkout's absolute filesystem prefix
