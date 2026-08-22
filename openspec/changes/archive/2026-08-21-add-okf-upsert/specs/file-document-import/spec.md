## MODIFIED Requirements

### Requirement: Local source imports include origin metadata

The system SHALL add generalized `origin` metadata to every emitted document derived from a local file source. For local file inputs, `origin.scheme` SHALL be `file`, `origin.path` SHALL be the directory path relative to the working directory, and `origin.filename` SHALL be the final file name. A file at the working-directory root SHALL use `./` as its relative origin path.

#### Scenario: A local file document includes origin metadata

- **WHEN** the user imports a local Markdown, text, YAML, JSON, NDJSON, JSONL, Toon, or converted document file
- **THEN** the emitted document includes an `origin` object
- **AND** `origin.scheme` is `file`
- **AND** `origin.path` is relative to the working directory
- **AND** `origin.filename` is the file's final path component

#### Scenario: A multi-source input includes every source origin

- **WHEN** a multi-source input resolves to more than one regular file
- **THEN** each emitted document includes `origin.scheme` equal to `file`
- **AND** each emitted document includes its relative `origin.path`
- **AND** each emitted document includes its `origin.filename`

#### Scenario: A single-source input includes its source origin

- **WHEN** a single-source input resolves to one direct file
- **THEN** the emitted document includes `origin.scheme` equal to `file`
- **AND** the emitted document includes a relative `origin.path`
- **AND** the emitted document includes its `origin.filename`

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
