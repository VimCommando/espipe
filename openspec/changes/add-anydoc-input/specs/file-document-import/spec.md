## MODIFIED Requirements

### Requirement: Local file inputs import documents by file format

The system SHALL accept one or more local file inputs and import each regular file according to its file format, including conversion through anydoc for supported local non-text formats.

#### Scenario: Single Markdown file is imported

- **WHEN** the user runs `espipe` with a local Markdown file input
- **THEN** the system emits one document for that Markdown file

#### Scenario: Single supported non-text file is imported

- **WHEN** the user runs `espipe` with a local PDF or other supported anydoc file input
- **THEN** the system converts the file to Markdown
- **AND** the system emits one document for that file

#### Scenario: Shell-expanded Markdown files are imported

- **WHEN** the user's shell expands a file pattern into multiple Markdown file arguments before `espipe` starts
- **THEN** the system treats each file argument as an input
- **AND** it emits one document for each regular file

#### Scenario: Multiple input positionals are provided

- **WHEN** the user provides more than two positional arguments
- **THEN** the final positional argument is treated as the output URI
- **AND** every preceding positional argument is treated as an input

### Requirement: Recursive glob inputs import matching files

The system SHALL accept local glob input patterns, including recursive `**` patterns, and import each matched regular file according to its file format, including anydoc conversion for supported non-text files.

#### Scenario: Recursive Markdown glob is imported

- **WHEN** the user runs `espipe` with a local input pattern of `**/*.md`
- **THEN** the system expands the pattern recursively
- **AND** it emits one document for each matched Markdown file

#### Scenario: Recursive PDF glob is imported

- **WHEN** the user runs `espipe` with a local input pattern of `**/*.pdf`
- **THEN** the system expands the pattern recursively
- **AND** it converts and emits one document for each matched PDF file

#### Scenario: Glob matches no files

- **WHEN** the user provides a local glob pattern that matches no regular files
- **THEN** startup fails before sending any output
- **AND** the error identifies that the glob matched no files

#### Scenario: Glob matches directories

- **WHEN** a local glob pattern matches both regular files and directories
- **THEN** the system imports the matched regular files
- **AND** it does not emit documents for matched directories

### Requirement: Multi-file imports include origin metadata

The system SHALL add one `origin` object to emitted documents when file-document input resolves more than one regular file or uses a glob pattern. The object SHALL contain `scheme`, `authority`, `path`, `query`, `fragment`, and `filename` fields, and the system SHALL NOT emit the legacy `file.path` or `file.name` metadata.

#### Scenario: Multiple files are imported

- **WHEN** file-document input resolves to more than one regular file
- **THEN** each emitted document includes the source file's `origin` object
- **AND** each emitted document does not include a `file` object

#### Scenario: Single direct file is imported

- **WHEN** file-document input resolves to one direct file without glob resolution
- **THEN** the emitted document does not include `origin`
- **AND** it does not include a `file` object

#### Scenario: A glob resolves one or more files

- **WHEN** file-document input uses a local glob pattern
- **THEN** each emitted document includes `origin.scheme` equal to `file`
- **AND** `origin.path` identifies the containing directory
- **AND** `origin.filename` identifies the source file
- **AND** absent `origin.authority`, `origin.query`, and `origin.fragment` fields are omitted

### Requirement: Binary files are rejected

The system SHALL convert local binary files recognized by anydoc into Markdown documents. It SHALL reject file-document inputs that are neither valid UTF-8 text nor recognized supported anydoc formats.

#### Scenario: Supported binary file is matched

- **WHEN** a file-document input resolves to a PDF or supported office/container file recognized by anydoc
- **THEN** the system converts the file to Markdown
- **AND** it emits a document for the converted content

#### Scenario: Unrecognized binary file is matched

- **WHEN** a file-document input resolves to a binary file that anydoc does not recognize
- **THEN** importing that file fails
- **AND** the error identifies the file as unsupported binary or invalid UTF-8 input

#### Scenario: Valid UTF-8 text remains supported

- **WHEN** a file-document input resolves to an unknown-extension file whose contents are valid UTF-8
- **THEN** the emitted document contains the full file content in the configured `content.<field_name>` field
