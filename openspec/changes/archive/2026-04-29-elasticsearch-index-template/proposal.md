## Why

Users often need index mappings and settings in place before bulk indexing begins. A template preflight option lets `espipe` prepare the target Elasticsearch cluster deterministically and fail before ingesting documents if Elasticsearch rejects the template.

## What Changes

- Add a `--template <path>` command-line option for Elasticsearch outputs.
- Add a `--template-name <name>` override, defaulting to the template file name without its extension.
- Add a `--template-overwrite <bool>` option that defaults to `true`.
- Read the template file as `.json`, `.jsonc`, or `.json5` before sending any documents.
- Send the parsed template as JSON to the Elasticsearch cluster before the first bulk request.
- Warn when the template `index_patterns` do not match the target output index name.
- Abort the run if the template file is unreadable, invalid for its template format, or rejected by Elasticsearch.
- Keep non-Elasticsearch outputs and runs without `--template` unchanged.

## Capabilities

### New Capabilities

- `elasticsearch-index-template`: Defines CLI-driven Elasticsearch index template preflight behavior before document ingestion.

### Modified Capabilities

None.

## Impact

- Affected CLI surface: adds `--template <path>`, `--template-name <name>`, and `--template-overwrite <bool>`.
- Affected output setup: Elasticsearch outputs need template parsing and a preflight request before bulk worker ingestion starts.
- Affected failure behavior: template validation or rejection aborts the run before documents are sent.
- Tests should cover strict JSON and commented template file parsing, template name derivation and override, overwrite behavior, index-pattern warnings, request ordering, Elasticsearch rejection, and rejection of `--template` with non-Elasticsearch outputs.
