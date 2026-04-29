## Why

Users need to ingest directory-shaped content collections, such as documentation trees, without pre-converting each file into NDJSON. File imports let `espipe` treat provided or matched files as Elasticsearch documents according to their format while preserving text content and Markdown metadata.

## What Changes

- Accept one or more local file inputs and import each file according to extension-specific document semantics.
- Support recursive glob input patterns, including patterns such as `**/*.md`, while also working when the user's shell expands the pattern into file arguments before `espipe` starts.
- Add Markdown-specific parsing for YAML frontmatter blocks.
- Add Markdown frontmatter fields under `content.<field>`.
- Add extension-aware file document handling for Markdown, text, YAML, JSON, NDJSON, and JSONL files.
- Store text content in a configurable `content.<field_name>` field.
- Add a `--content <field_name>` command-line argument that controls the content subfield name and defaults to `body`.
- Add `file.path` and `file.name` fields when an import resolves multiple files.
- Reject binary files, ambiguous inputs, and invalid file import inputs with actionable errors.

## Capabilities

### New Capabilities

- `file-document-import`: Defines one-document-per-file imports from plain-text files and glob patterns, Markdown frontmatter mapping, and configurable content field naming.

### Modified Capabilities

None.

## Impact

- Affected CLI surface: input argument handling for one or more file inputs and the new `--content <field_name>` option.
- Affected ingest path: input discovery, file reading, document construction, and output dispatch.
- Affected formats: Markdown files gain frontmatter-aware document extraction under `content.*`; text files use the configured `content.<field>` for full file contents; YAML files become `content.*` metadata documents; `.json` files are treated as one whole-file JSON object; `.ndjson` and `.jsonl` remain per-line streaming formats.
- Likely dependencies: a glob walking implementation and a YAML frontmatter parser or existing YAML parsing support.
- Tests should cover shell-expanded file lists, recursive matching, deterministic file ordering, binary rejection, Markdown frontmatter extraction, custom content field names, extension-specific handling, and error cases.
