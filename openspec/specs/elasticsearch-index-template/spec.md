## Purpose

Define how `espipe` installs Elasticsearch composable index templates before bulk indexing.

## Requirements

### Requirement: Template option installs an Elasticsearch index template
The system SHALL accept `--template <path>` for Elasticsearch outputs and send the config file as a composable index template JSON request before sending any bulk document request.

#### Scenario: Template is installed before bulk indexing
- **WHEN** the user runs `espipe` with an Elasticsearch output and `--template template.json`
- **THEN** the system reads `template.json`
- **AND** it sends the parsed template to Elasticsearch as JSON before the first `_bulk` request
- **AND** it sends document batches only after Elasticsearch accepts the template request

#### Scenario: Default template name is derived from file name
- **WHEN** the user passes `--template ./templates/logs-docs.json`
- **THEN** the system sends the template request to `/_index_template/logs-docs`

#### Scenario: Template name is overridden
- **WHEN** the user passes `--template ./templates/logs-docs.json --template-name custom-template`
- **THEN** the system sends the template request to `/_index_template/custom-template`

#### Scenario: Empty template name is rejected
- **WHEN** the derived template name or `--template-name` value is empty
- **THEN** startup fails before any documents are sent
- **AND** the error explains that the template name must be non-empty

### Requirement: Template overwrite behavior is configurable
The system SHALL overwrite existing composable index templates by default and SHALL use Elasticsearch create-only template installation when `--template-overwrite=false`.

#### Scenario: Template overwrite defaults to true
- **WHEN** the user passes `--template template.json` without `--template-overwrite`
- **THEN** the system sends `PUT /_index_template/{template_name}` using overwrite semantics
- **AND** an existing template with the same name can be replaced if Elasticsearch authorizes the request

#### Scenario: Template overwrite is disabled
- **WHEN** the user passes `--template template.json --template-overwrite=false`
- **THEN** the system sends `POST /_index_template/{template_name}?create=true` with Elasticsearch create-only semantics
- **AND** the run fails if Elasticsearch reports that the template already exists
- **AND** no bulk document request is sent

### Requirement: Only composable index templates are supported
The system SHALL send template requests only to the Elasticsearch composable index template API.

#### Scenario: Template API path is used
- **WHEN** the user passes `--template template.json`
- **THEN** the system sends the request to the `/_index_template/{template_name}` API
- **AND** it does not send a request to the legacy `/_template/{template_name}` API

### Requirement: Template files must be valid supported template syntax
The system SHALL validate `.json`, `.jsonc`, `.json5`, `.yml`, and `.yaml` template files before sending them to Elasticsearch, and SHALL preserve strict JSON parsing for template files with other extensions for backwards compatibility.

#### Scenario: Template file is unreadable
- **WHEN** the user passes `--template` with a path that cannot be read
- **THEN** startup fails before any documents are sent
- **AND** the error identifies the template path
- **AND** the error is written to stderr

#### Scenario: Template file is invalid JSON
- **WHEN** the user passes `--template` with a file that is not valid JSON
- **THEN** startup fails before any documents are sent
- **AND** the error identifies the template path and JSON parse failure
- **AND** the error is written to stderr

#### Scenario: JSONC template contains comments
- **WHEN** the user passes `--template template.jsonc`
- **AND** the template contains C-style block comments
- **THEN** the system parses the template successfully
- **AND** it sends a valid JSON request body to Elasticsearch

#### Scenario: JSON5 template is provided
- **WHEN** the user passes `--template template.json5`
- **THEN** the system parses the template using JSON5-compatible syntax
- **AND** it sends a valid JSON request body to Elasticsearch

#### Scenario: YAML template file is provided
- **WHEN** the user passes `--template template.yml`
- **THEN** the system parses the YAML template successfully
- **AND** it sends a valid JSON request body to Elasticsearch

#### Scenario: Template file extension matching is case-insensitive
- **WHEN** the user passes `--template template.YAML`
- **THEN** the system treats the template file as YAML
- **AND** it sends a valid JSON request body to Elasticsearch

#### Scenario: Template file with unknown extension contains strict JSON
- **WHEN** the user passes `--template template.txt`
- **AND** the template file contains valid strict JSON
- **THEN** the system parses the template successfully
- **AND** it sends a valid JSON request body to Elasticsearch

#### Scenario: Commented template syntax is invalid
- **WHEN** the user passes `--template` with a `.jsonc` or `.json5` file that cannot be parsed
- **THEN** startup fails before any documents are sent
- **AND** the error identifies the template path and parse failure
- **AND** the error is written to stderr

### Requirement: Template index patterns are checked against target index
The system SHALL inspect template `index_patterns` and warn when no declared pattern matches the output target index name.

#### Scenario: Multi-target expression includes target index
- **WHEN** the output target index is `test3`
- **AND** the template JSON contains `index_patterns` with `test*`
- **THEN** the system treats the target index as matched
- **AND** it sends the template request without an index-pattern mismatch warning

#### Scenario: Multi-target exclusion removes target index
- **WHEN** the output target index is `test3`
- **AND** the template JSON contains `index_patterns` with `test*,-test3`
- **THEN** the system treats the target index as unmatched
- **AND** it emits an index-pattern mismatch warning

#### Scenario: Later include overrides earlier exclusion
- **WHEN** the output target index is `test3`
- **AND** the template JSON contains `index_patterns` with `test3*,-test3,test*`
- **THEN** the system treats the target index as matched
- **AND** it sends the template request without an index-pattern mismatch warning

#### Scenario: Index pattern matches target index
- **WHEN** the output target index is `logs-docs`
- **AND** the template JSON contains `index_patterns` that match `logs-docs`
- **THEN** the system sends the template request without an index-pattern mismatch warning

#### Scenario: Index pattern does not match target index
- **WHEN** the output target index is `logs-docs`
- **AND** the template JSON contains `index_patterns` that do not match `logs-docs`
- **THEN** the system emits a warning before sending documents
- **AND** the warning is written to stderr
- **AND** it does not fail solely because of the mismatch

#### Scenario: Index patterns cannot be checked
- **WHEN** the template JSON omits `index_patterns` or uses an unexpected `index_patterns` shape
- **THEN** the system emits a warning that the target index match could not be verified
- **AND** the warning is written to stderr
- **AND** Elasticsearch remains responsible for accepting or rejecting the template

#### Scenario: Index pattern syntax is invalid for local check
- **WHEN** the template JSON contains an `index_patterns` expression with a lone `-`
- **THEN** the system emits a warning that the target index match could not be verified
- **AND** the warning is written to stderr
- **AND** Elasticsearch remains responsible for accepting or rejecting the template

### Requirement: Template rejection aborts ingestion
The system SHALL abort the run when Elasticsearch rejects the template request.

#### Scenario: Elasticsearch rejects template
- **WHEN** Elasticsearch responds to the template request with a non-2xx status
- **THEN** the system fails the run
- **AND** no bulk document request is sent
- **AND** the error includes the response status and available Elasticsearch error details
- **AND** the error is written to stderr

#### Scenario: Template request cannot be completed
- **WHEN** the template request fails because of authentication, TLS, DNS, timeout, or transport error
- **THEN** the system fails the run
- **AND** no bulk document request is sent
- **AND** the error is written to stderr

### Requirement: Template argument failures occur before input access
The system SHALL validate template-related arguments and output compatibility before opening or reading input content.

#### Scenario: Template option is invalid
- **WHEN** the user provides invalid template-related arguments
- **THEN** startup fails before opening or reading input content
- **AND** the error is written to stderr

#### Scenario: Template option is incompatible with output
- **WHEN** the user provides template-related arguments with a non-Elasticsearch output
- **THEN** startup fails before opening or reading input content
- **AND** the error is written to stderr

### Requirement: Template option only applies to Elasticsearch outputs
The system SHALL reject template-related options when the selected output is not Elasticsearch.

#### Scenario: Template is used with file output
- **WHEN** the user passes `--template template.json` with a file output
- **THEN** startup fails before reading input documents
- **AND** the error explains that `--template` requires an Elasticsearch output
- **AND** the error is written to stderr

#### Scenario: Template is used with stdout output
- **WHEN** the user passes `--template template.json` with stdout output
- **THEN** startup fails before reading input documents
- **AND** the error explains that `--template` requires an Elasticsearch output
- **AND** the error is written to stderr

#### Scenario: Template name is used without Elasticsearch output
- **WHEN** the user passes `--template-name custom-template` with a file or stdout output
- **THEN** startup fails before reading input documents
- **AND** the error explains that template options require an Elasticsearch output
- **AND** the error is written to stderr

#### Scenario: Template name is used without template path
- **WHEN** the user passes `--template-name custom-template` without `--template`
- **THEN** startup fails before reading input documents
- **AND** the error explains that `--template-name` requires `--template`
- **AND** the error is written to stderr

#### Scenario: Template overwrite is used without template path
- **WHEN** the user passes `--template-overwrite=false` without `--template`
- **THEN** startup fails before reading input documents
- **AND** the error explains that `--template-overwrite` requires `--template`
- **AND** the error is written to stderr

### Requirement: Runs without template remain unchanged
The system SHALL preserve existing output behavior when `--template` is not provided.

#### Scenario: Elasticsearch output without template
- **WHEN** the user runs `espipe` with an Elasticsearch output and no `--template`
- **THEN** the system does not send an index template request
- **AND** document bulk indexing starts using the existing output flow

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
