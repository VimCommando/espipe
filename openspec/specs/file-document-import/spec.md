## Purpose

Define how `espipe` imports local files, shell-expanded file lists, and local glob patterns as JSON documents.

## Requirements

### Requirement: Local file inputs import documents by file format
The system SHALL accept one or more local file inputs and import each regular file according to its file format, including conversion through anydoc for supported local non-text formats.

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

#### Scenario: Single supported non-text file is imported
- **WHEN** the user runs `espipe` with a local PDF or other supported anydoc file input
- **THEN** the system converts the file to Markdown
- **AND** the system emits one document for that file

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

### Requirement: Batch file errors do not abort later imports
When a batch of local file-document inputs encounters a per-file read or conversion error, the system SHALL log a warning identifying the source and error, skip that file, and continue importing the remaining files. A direct single-file import SHALL retain its fatal error behavior.

#### Scenario: A file fails during a batch import
- **WHEN** one file cannot be read or converted during a multi-file or glob import
- **THEN** the system logs a warning identifying the failed source
- **AND** it does not emit a synthetic document for that file
- **AND** it continues importing later files

#### Scenario: A direct file fails during import
- **WHEN** the only direct file input cannot be read or converted
- **THEN** ingestion fails with a diagnostic identifying the source and error

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

### Requirement: Local source imports include origin metadata
The system SHALL add one `origin` object to every emitted document derived from a local file source. For local file inputs, `origin.scheme` SHALL be `file`, `origin.path` SHALL be the directory path relative to the working directory, and `origin.filename` SHALL be the final file name. A file at the working-directory root SHALL use `./` as its relative origin path. URI fields without values SHALL be omitted. The system SHALL NOT emit the legacy `file.path` or `file.name` metadata.

#### Scenario: A local file document includes origin metadata
- **WHEN** the user imports a local Markdown, text, YAML, JSON, NDJSON, JSONL, Toon, or converted document file
- **THEN** the emitted document includes an `origin` object
- **AND** `origin.scheme` is `file`
- **AND** `origin.path` is relative to the working directory
- **AND** `origin.filename` is the file's final path component

#### Scenario: A multi-source input includes every source origin
- **WHEN** a multi-source input resolves to more than one regular file
- **THEN** each emitted document includes its relative `origin.path` and `origin.filename`
- **AND** each emitted document does not include a `file` object

#### Scenario: A single-source input includes its source origin
- **WHEN** a single-source input resolves to one direct file
- **THEN** the emitted document includes its relative `origin.path` and `origin.filename`
- **AND** the emitted document does not include a `file` object

#### Scenario: A glob resolves one or more files
- **WHEN** file-document input uses a local glob pattern
- **THEN** each emitted document includes `origin.scheme` equal to `file`
- **AND** `origin.path` identifies the containing directory
- **AND** `origin.filename` identifies the source file
- **AND** absent `origin.authority`, `origin.query`, and `origin.fragment` fields are omitted

#### Scenario: Split applies independently to each multi-source file
- **WHEN** a multi-source input resolves to multiple local files
- **AND** the user passes `--split <JSON_POINTER>`
- **THEN** the system applies the split operation independently to each source file
- **AND** every emitted document retains the origin of the file that produced it

#### Scenario: Multi-source input rejects files outside the working directory
- **WHEN** a multi-source input contains a file outside the working directory
- **AND** the file is not reached through an allowed symlink
- **THEN** input validation fails before any document is emitted

#### Scenario: Multi-source discovery skips symlinks by default
- **WHEN** a multi-source input resolves a file whose path contains a symlink component
- **AND** the user does not pass `--symlinks`
- **THEN** the symlink path is omitted before source cardinality is calculated

#### Scenario: Multi-source discovery can fail on symlinks
- **WHEN** a multi-source input resolves a file whose path contains a symlink component
- **AND** the user passes `--symlinks=fail`
- **THEN** input validation fails before any document is emitted

#### Scenario: Multi-source discovery can follow external symlinks
- **WHEN** a multi-source input resolves a file whose path contains a symlink component
- **AND** the user passes `--symlinks=follow`
- **THEN** the file is imported even when the symlink target is outside the working directory
- **AND** its `origin.path` and `origin.filename` use the supplied symlink path
- **AND** generated identity uses the supplied symlink path rather than the target's absolute path

#### Scenario: Multi-source discovery skips hidden paths by default
- **WHEN** a multi-source input resolves a path containing a dot-prefixed file or directory component
- **AND** the user does not pass `--hidden`
- **THEN** the hidden path is omitted before source cardinality is calculated

#### Scenario: Multi-source discovery can include hidden paths
- **WHEN** a multi-source input resolves a path containing a dot-prefixed file or directory component
- **AND** the user passes `--hidden=include`
- **THEN** the hidden path is eligible for import

#### Scenario: Multi-source discovery can fail on hidden paths
- **WHEN** a multi-source input resolves a path containing a dot-prefixed file or directory component
- **AND** the user passes `--hidden=fail`
- **THEN** input validation fails before any document is emitted

#### Scenario: Discovery filtering determines source cardinality
- **WHEN** a multi-source discovery input contains candidates that are skipped by the symlink or hidden policy
- **THEN** skipped candidates are removed before single-source or multi-source classification
- **AND** generated-ID defaults use the remaining source count

#### Scenario: Single-source input may reference an external file
- **WHEN** a single-source input references a file outside the working directory
- **THEN** the input is not rejected solely because it is outside the working directory
- **AND** its origin remains relative to the working directory

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

#### Scenario: Remote inputs preserve URI origin metadata
- **WHEN** an HTTP or HTTPS CSV, NDJSON, or Toon input is imported
- **THEN** each emitted document includes the source URI's available origin fields
- **AND** the values reflect the source URI rather than the temporary download file

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

### Requirement: Toon files stream one document per Toon document
The system SHALL import `.toon` files as structured Toon input where each decoded Toon object emits one JSON object document.

#### Scenario: Toon file is imported
- **WHEN** a file-document input resolves to a `.toon` file
- **THEN** the system parses the file using the Toon input reader
- **AND** each decoded Toon object emits one document

#### Scenario: Toon file is included with multiple file inputs
- **WHEN** file-document input contains a `.toon` file and other supported files
- **THEN** the `.toon` file participates in the existing deterministic file input order
- **AND** documents decoded from that `.toon` file are emitted at that file's position in the ordered input sequence

#### Scenario: Toon file contains a non-object document
- **WHEN** a `.toon` file contains a document that decodes to a non-object value
- **THEN** importing that file fails
- **AND** the invalid Toon document is not sent to any output

### Requirement: Binary files are rejected
The system SHALL convert local binary files recognized by anydoc into Markdown documents. It SHALL reject file-document inputs that are neither valid UTF-8 text nor recognized supported anydoc formats.

#### Scenario: Binary file is matched
- **WHEN** a file-document input resolves to a file whose contents are not valid UTF-8 text
- **THEN** importing that file fails
- **AND** the error identifies the file as non-text or invalid UTF-8

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

### Requirement: File import diagnostics are written to stderr
The system SHALL write user-facing file import warnings and errors to stderr.

#### Scenario: File import fails
- **WHEN** file-document input fails because of an invalid argument, invalid file contents, or unsupported file shape
- **THEN** the diagnostic is written to stderr
- **AND** no documents are sent after the failure

### Requirement: Duplicate Markdown frontmatter keys are warnings
The system SHALL warn and continue when Markdown frontmatter contains a duplicate mapping key. The last value for that key SHALL be used in the emitted content. Other invalid frontmatter, including non-mapping frontmatter, malformed YAML, and conflicts with the configured content field, SHALL remain fatal.

#### Scenario: Duplicate frontmatter keys use the last value
- **WHEN** a Markdown file contains the same frontmatter key more than once
- **THEN** the system emits a warning identifying the source and duplicate key
- **AND** the Markdown document is imported successfully
- **AND** the last value for the key is present in the emitted content

#### Scenario: Invalid frontmatter remains rejected
- **WHEN** Markdown frontmatter is malformed or is not a mapping
- **THEN** input processing fails with an invalid-frontmatter error
