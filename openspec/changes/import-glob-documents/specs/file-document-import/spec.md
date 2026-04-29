## ADDED Requirements

### Requirement: Local file inputs import documents by file format
The system SHALL accept one or more local file inputs and import each regular file according to its file format.

#### Scenario: Single Markdown file is imported
- **WHEN** the user runs `espipe` with a local Markdown file input
- **THEN** the system emits one document for that Markdown file

#### Scenario: Shell-expanded Markdown files are imported
- **WHEN** the user's shell expands a file pattern into multiple Markdown file arguments before `espipe` starts
- **THEN** the system treats each file argument as an input
- **AND** it emits one document for each regular file

#### Scenario: Multiple input positionals are provided
- **WHEN** the user provides more than two positional arguments
- **THEN** the final positional argument is treated as the output URI
- **AND** every preceding positional argument is treated as an input

### Requirement: Recursive glob inputs import matching files
The system SHALL accept local glob input patterns, including recursive `**` patterns, and import each matched regular file according to its file format.

#### Scenario: Recursive Markdown glob is imported
- **WHEN** the user runs `espipe` with a local input pattern of `**/*.md`
- **THEN** the system expands the pattern recursively
- **AND** it emits one document for each matched Markdown file

#### Scenario: Glob matches no files
- **WHEN** the user provides a local glob pattern that matches no regular files
- **THEN** startup fails before sending any output
- **AND** the error identifies that the glob matched no files

#### Scenario: Glob matches directories
- **WHEN** a local glob pattern matches both regular files and directories
- **THEN** the system imports the matched regular files
- **AND** it does not emit documents for matched directories

### Requirement: File document import order is deterministic
The system SHALL process file-document inputs in deterministic lexicographic path order after combining concrete file inputs and glob matches.

#### Scenario: Multiple files are imported
- **WHEN** file-document input contains multiple files
- **THEN** the emitted documents follow lexicographic order by file path

#### Scenario: Same file appears more than once
- **WHEN** a file is provided directly and also matched by a glob pattern
- **THEN** the system emits at most one document for that file

### Requirement: File documents store content in a configurable field
The system SHALL store imported text content in the `content.<field_name>` field named by the `--content <field_name>` command-line argument, defaulting to `content.body`.

#### Scenario: Default content field is used
- **WHEN** the user imports files without passing `--content`
- **THEN** each emitted document stores the file content in the `content.body` field

#### Scenario: Custom content field is used
- **WHEN** the user imports files with `--content markdown`
- **THEN** each emitted document stores the file content in the `content.markdown` field
- **AND** it does not add a `content.body` field unless that field is provided by format-specific metadata

#### Scenario: Empty content field is rejected
- **WHEN** the user passes an empty value for `--content`
- **THEN** argument parsing or startup fails
- **AND** no documents are sent

#### Scenario: Dotted content field is rejected
- **WHEN** the user passes a `--content` value containing `.`
- **THEN** argument parsing or startup fails
- **AND** no documents are sent

### Requirement: Multi-file imports include file metadata
The system SHALL add `file.path` and `file.name` fields to emitted documents only when file-document input resolves more than one regular file.

#### Scenario: Multiple files are imported
- **WHEN** file-document input resolves to more than one regular file
- **THEN** each emitted document includes `file.path`
- **AND** each emitted document includes `file.name`

#### Scenario: Single direct file is imported
- **WHEN** file-document input resolves to one direct file without glob or multi-file resolution
- **THEN** the emitted document does not include `file.path`
- **AND** it does not include `file.name`

### Requirement: Markdown frontmatter becomes document fields
The system SHALL parse a leading YAML frontmatter block in Markdown files and add each frontmatter field under the `content` object.

#### Scenario: Markdown file has YAML frontmatter
- **WHEN** a matched Markdown file starts with a YAML frontmatter block delimited by `---`
- **THEN** each top-level field in the frontmatter mapping is added as `content.<metadata_field>`
- **AND** the Markdown content after the closing delimiter is stored in the configured `content.<field_name>` field

#### Scenario: Markdown file has no frontmatter
- **WHEN** a matched Markdown file does not start with a YAML frontmatter block
- **THEN** the emitted document contains the full Markdown file content in the configured `content.<field_name>` field
- **AND** no frontmatter fields are added

#### Scenario: Frontmatter root is not a mapping
- **WHEN** a Markdown file contains a leading YAML frontmatter block whose root value is not a mapping
- **THEN** importing that file fails
- **AND** the error identifies the file with invalid frontmatter

### Requirement: Markdown content field conflicts are rejected
The system SHALL reject Markdown documents where a frontmatter field uses the same subfield name as the configured `content.<field_name>` field.

#### Scenario: Frontmatter conflicts with default body field
- **WHEN** a Markdown file frontmatter includes a `body` field
- **AND** the user did not pass `--content`
- **THEN** importing that file fails
- **AND** the system does not overwrite either the frontmatter field or the Markdown content

#### Scenario: Frontmatter conflicts with custom content field
- **WHEN** a Markdown file frontmatter includes a field matching the value passed to `--content`
- **THEN** importing that file fails
- **AND** the error identifies the conflicting field name

### Requirement: Non-Markdown text files import full file content
The system SHALL import `.txt`, `.text`, `.log`, and unknown UTF-8 files as documents containing the full file contents in the configured `content.<field_name>` field.

#### Scenario: Text file is matched by glob
- **WHEN** a local glob input matches a `.txt` file
- **THEN** the emitted document contains the full text file content in the configured `content.<field_name>` field
- **AND** no Markdown frontmatter parsing is applied

#### Scenario: Text file is provided directly
- **WHEN** the user provides a local `.txt` file as a file-document input
- **THEN** the emitted document contains the full text file content in the configured `content.<field_name>` field
- **AND** no Markdown frontmatter parsing is applied

#### Scenario: Unknown UTF-8 file is imported
- **WHEN** a file-document input resolves to a file with an unknown extension and valid UTF-8 contents
- **THEN** the emitted document contains the full file content in the configured `content.<field_name>` field

### Requirement: YAML files import as JSON object documents
The system SHALL import `.yml` and `.yaml` files by parsing each file as one YAML document and converting a mapping root into `content.*` fields.

#### Scenario: YAML mapping file is imported
- **WHEN** a file-document input resolves to a `.yml` or `.yaml` file whose root is a YAML mapping
- **THEN** the emitted document contains the mapping fields converted to JSON under `content.<metadata_field>`
- **AND** it does not wrap the YAML document in the configured `content.<field_name>` field

#### Scenario: YAML root is not a mapping
- **WHEN** a `.yml` or `.yaml` file root is a scalar, sequence, or null value
- **THEN** importing that file fails
- **AND** the error identifies the file with invalid YAML document shape

### Requirement: JSON files import as one whole-file object
The system SHALL import `.json` files only by parsing the full file as one JSON object document.

#### Scenario: JSON object file is imported
- **WHEN** a file-document input resolves to a `.json` file containing one JSON object
- **THEN** the emitted document is that JSON object
- **AND** it does not wrap the JSON object in the configured `content.<field_name>` field

#### Scenario: JSON array file is rejected
- **WHEN** a file-document input resolves to a `.json` file containing a JSON array
- **THEN** importing that file fails
- **AND** the system does not split the array into documents

#### Scenario: JSON file is not one object
- **WHEN** a `.json` file cannot be parsed as one whole-file JSON object
- **THEN** importing that file fails
- **AND** the error reports that `.json` inputs must contain one JSON object

### Requirement: NDJSON and JSONL files stream one document per line
The system SHALL import `.ndjson` and `.jsonl` files as line-delimited JSON where each non-empty line emits one JSON object document.

#### Scenario: NDJSON file is imported
- **WHEN** a file-document input resolves to a `.ndjson` file
- **THEN** each non-empty line is parsed as a JSON object
- **AND** each parsed line emits one document

#### Scenario: JSONL file is imported
- **WHEN** a file-document input resolves to a `.jsonl` file
- **THEN** each non-empty line is parsed as a JSON object
- **AND** each parsed line emits one document

#### Scenario: NDJSON line is not an object
- **WHEN** a `.ndjson` or `.jsonl` file contains a line that is valid JSON but not a JSON object
- **THEN** importing that file fails
- **AND** the error identifies the file and line as invalid

### Requirement: Binary files are rejected
The system SHALL reject file-document inputs that are not valid UTF-8 text.

#### Scenario: Binary file is matched
- **WHEN** a file-document input resolves to a file whose contents are not valid UTF-8 text
- **THEN** importing that file fails
- **AND** the error identifies the file as non-text or invalid UTF-8

### Requirement: File import diagnostics are written to stderr
The system SHALL write user-facing file import warnings and errors to stderr.

#### Scenario: File import fails
- **WHEN** file-document input fails because of an invalid argument, invalid file contents, or unsupported file shape
- **THEN** the diagnostic is written to stderr
- **AND** no documents are sent after the failure
