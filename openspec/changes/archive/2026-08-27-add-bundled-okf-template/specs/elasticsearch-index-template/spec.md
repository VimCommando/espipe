## ADDED Requirements

### Requirement: Underscore template values select bundled templates
The system SHALL interpret a `--template` value beginning with `_` as a bundled template selector and SHALL continue to interpret every other value as a filesystem path.

#### Scenario: OKF bundled template is selected
- **WHEN** the user passes `--template _okf` with an Elasticsearch output
- **THEN** the system resolves the bundled `_okf` template without reading a template file from the filesystem
- **AND** template preflight completes before any input content is opened or any bulk request is sent

#### Scenario: File-backed template behavior is preserved
- **WHEN** the user passes `--template templates/_okf.json`
- **THEN** the system treats the value as a filesystem path because the complete value does not begin with `_`
- **AND** it preserves existing file parsing, naming, index-pattern warning, and installation behavior

#### Scenario: Unknown bundled template is rejected
- **WHEN** the user passes a bundled selector that the executable does not contain
- **THEN** startup fails before input access and bulk ingestion
- **AND** the error identifies the unknown selector and lists the available bundled template names

### Requirement: Bundled templates have default Elasticsearch names
The system SHALL associate every bundled template selector with a default Elasticsearch template name and SHALL use `--template-name` as an override. `_okf` SHALL default to `open-knowledge-format`. Before installing or updating a bundled template, the system SHALL request the selected name from Elasticsearch.

#### Scenario: OKF uses its default name
- **WHEN** the user passes `--template _okf` and the output target index is `team-knowledge`
- **AND** Elasticsearch reports that `open-knowledge-format` does not exist
- **THEN** the system adds `team-knowledge` to the bundled template's `index_patterns`
- **AND** it creates the template as `open-knowledge-format`
- **AND** it creates no target-specific template

#### Scenario: Bundled template name is overridden
- **WHEN** the user passes `--template _okf --template-name team-okf`
- **THEN** the system requests, creates, or updates `team-okf`
- **AND** it does not request or write `open-knowledge-format`

#### Scenario: Future bundled template name is overridden
- **WHEN** the executable contains a bundled selector `_catalog`
- **AND** the user passes `--template _catalog --template-name company-catalog`
- **THEN** the system requests, creates, or updates `company-catalog`
- **AND** the selected `_catalog` asset supplies the initial template body when `company-catalog` is absent

#### Scenario: Selected template lookup fails
- **WHEN** the request for the default or overridden template name fails because of authentication, TLS, transport, timeout, or an unexpected Elasticsearch response
- **THEN** startup fails before input access and bulk ingestion
- **AND** the error identifies the template lookup failure

### Requirement: Existing bundled template gains new target indices
When the selected default or overridden Elasticsearch template exists, the system SHALL read its stored composable template body and SHALL append the exact output index name to its `index_patterns` only when that exact value is absent. The system SHALL preserve the existing pattern order and all other stored template fields.

#### Scenario: New target index is appended
- **WHEN** the user passes `--template _okf` for target index `team-knowledge`
- **AND** `open-knowledge-format` exists with `index_patterns` equal to `["company-knowledge"]`
- **THEN** the system updates the same template with `index_patterns` equal to `["company-knowledge", "team-knowledge"]`
- **AND** it preserves the template's existing mappings, settings, aliases, priority, version, metadata, and composed component references
- **AND** it sends no bulk request until Elasticsearch accepts the update

#### Scenario: Target index is already listed
- **WHEN** the user passes `--template _okf` for target index `team-knowledge`
- **AND** `open-knowledge-format` already lists the exact `team-knowledge` value in `index_patterns`
- **THEN** the system does not append a duplicate value
- **AND** it does not send a template update request
- **AND** bulk ingestion may proceed after preflight

#### Scenario: Existing patterns contain a wildcard match but not the exact index
- **WHEN** the target index is `team-knowledge`
- **AND** the existing `index_patterns` contains `team-*` but does not contain the exact value `team-knowledge`
- **THEN** the system appends `team-knowledge`

#### Scenario: Existing selected template cannot be merged
- **WHEN** the selected Elasticsearch template exists but its response lacks one unambiguous composable template body or has an invalid `index_patterns` value
- **THEN** startup fails before input access and bulk ingestion
- **AND** the system does not replace the malformed stored template with the bundled asset

#### Scenario: File template pattern remains user-owned
- **WHEN** the user selects a file-backed template whose patterns do not match the target index
- **THEN** the system does not add the target index to that template
- **AND** it preserves the existing mismatch warning behavior

### Requirement: Overwrite control applies to the selected bundled template name
The existing `--template-overwrite` control SHALL determine whether `espipe` may create or update the default or overridden Elasticsearch template selected for a bundled asset.

#### Scenario: Create-only mode creates a missing shared template
- **WHEN** the user passes `--template _okf --template-overwrite=false`
- **AND** the selected default or overridden Elasticsearch template does not exist
- **THEN** the system creates the selected template with Elasticsearch create-only semantics

#### Scenario: Create-only mode cannot append a missing target
- **WHEN** the user passes `--template _okf --template-overwrite=false`
- **AND** the selected default or overridden Elasticsearch template exists without the exact target index in `index_patterns`
- **THEN** startup fails before input access and bulk ingestion
- **AND** the system does not update the template
- **AND** the error explains that appending the target requires template overwrite behavior

#### Scenario: Create-only mode accepts an existing target
- **WHEN** the user passes `--template _okf --template-overwrite=false`
- **AND** the selected default or overridden Elasticsearch template already lists the exact target index
- **THEN** the system sends no template write request
- **AND** bulk ingestion may proceed after preflight

### Requirement: Bundled templates are present in distributed executables
The system SHALL include every bundled template asset in the compiled executable and SHALL resolve it at runtime without a source checkout, working-directory asset folder, or network access.

#### Scenario: Bundled template works outside the source tree
- **WHEN** a packaged `espipe` executable runs from a directory that contains no template assets
- **THEN** `--template _okf` resolves and installs the bundled template
