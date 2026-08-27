---
name: espipe
description: Use when the user wants to load, import, ingest, pipe, or bulk-send data with espipe, including local or remote CSV, NDJSON, JSON, Toon, Markdown, YAML, text, PDF, Office, OpenDocument, RTF, or EPUB inputs; file lists and recursive globs; JSON splitting; deterministic IDs; Elasticsearch bulk actions, pipelines, or templates; and Elasticsearch, file, or stdout outputs.
---

# Espipe Ingestion

Translate a clear ingestion request into one `espipe` command, run it, and report the exact command and result. Keep user-supplied paths and recursive globs intact.

## Inputs

Direct structured inputs:

- Local `.ndjson`, `.ndjson.gz`, `.json`, `.csv`, `.csv.gz`, and `.toon` files
- HTTP(S) `.csv`, `.ndjson`, `.json`, and `.toon` sources; remote inputs are unauthenticated
- `-` for NDJSON from `stdin`
- `file://` URIs for local files

CSV input uses the first row as headers and preserves all field values as JSON strings. Without `--split`, local and remote `.json` inputs are interpreted as line-delimited JSON.

Local file-document inputs:

- Markdown, plain text, YAML, JSON, NDJSON, JSONL, CSV, and Toon
- AnyDoc formats: `.doc`, `.docx`, `.docm`, `.odt`, `.pdf`, `.ppt`, `.pps`, `.pot`, `.pptx`, `.pptm`, `.ppsx`, `.ppsm`, `.rtf`, `.epub`, `.xls`, `.xlsx`, `.xlsm`, `.xlsb`, `.ods`, and `.odp`

AnyDoc converts local documents to GitHub-Flavored Markdown. Scanned or image-only PDFs are not OCR'd. File documents use `content.body` by default; use `--content <field>` when another content field is requested.

Multiple local paths and quoted recursive globs are supported. Keep patterns such as `'docs/**/*.pdf'` quoted so espipe performs discovery. The final positional argument is always the output URI.

`--split <JSON_POINTER>` selects an array or object from JSON and streams its children as documents. It works with local paths, `file://`, `stdin`, and HTTP(S) JSON inputs. Object keys become string `id` fields; array positions can contribute to generated IDs. Split is applied independently to each local source, and NDJSON cannot be used with split.

Local file documents and remote streaming inputs carry `origin` metadata. Local origins use a working-directory-relative `path`, `filename`, and `scheme: "file"`.

## Outputs

Elasticsearch outputs:

- `http://host:9200/index-name`
- `https://host:9200/index-name`
- `known-host:index-name`, resolved from `$ESPIPE_HOSTS` or `~/.espipe/hosts.yml`
- `env:/index-name`, resolved first from the process environment and then from `.env` using `ELASTIC_ES_URL` and optionally `ELASTIC_ES_API_KEY`

Other outputs:

- `-` for raw JSON lines on stdout
- Local `.ndjson` or `.ndjson.gz` files, including `file://` URIs

Elasticsearch targets must include an index name. File outputs truncate an existing target. Gzip file support is limited to `.csv.gz`, `.ndjson.gz` inputs and `.ndjson.gz` outputs.

Do not read `~/.espipe/hosts.yml` unless the user explicitly asks. Do not invent host aliases, URLs, credentials, or index names.

## Actions And Configuration

The default Elasticsearch action is `index`. Supported values are `create`, `index`, `update`, and `upsert`.

- `create` and `index` send the selected bulk operation.
- `update` sends `{ "doc": ... }` and requires each document to have an explicit string `_id` or a generated local-file ID.
- `upsert` sends `{ "doc": ..., "doc_as_upsert": true }` and has the same ID requirement.

`--generate-id=true|false` controls deterministic IDs for local files. Multi-source local inputs enable generation by default; single-source inputs require `--generate-id=true`. Generated IDs are stable and derived from the bundle, source path, and document discriminator, not file content or timestamps. Explicit top-level string `_id` values take precedence and are removed from the source body before transport. Non-file inputs never receive generated IDs.

For multi-source local discovery, `--symlinks=skip|follow|fail` and `--hidden=skip|include|fail` both default to `skip`. Sources outside the working directory are rejected by default; `--symlinks=follow` can follow an external symlink while preserving the supplied working-relative path.

For Elasticsearch outputs, configuration options are available before bulk ingestion:

- `--pipeline <path>` installs a JSON or YAML ingest pipeline
- `--pipeline-name <name>` overrides the pipeline name; `_none` disables a request-level default when compatible
- `--template <path|_name>` installs a file-backed composable index template or selects a template compiled into espipe
- `--template _okf` selects the bundled Open Knowledge Format v0.2 mapping; its default Elasticsearch name is `open-knowledge-format`
- `--template-name <name>` overrides a file-derived or bundled default template name
- `--template-overwrite=true|false` controls file-template replacement and bundled-template pattern updates

Bundled templates read their selected Elasticsearch template before ingestion. A missing template is created with the target index in `index_patterns`. An existing template gains the exact target index when absent while preserving its stored body. With `--template-overwrite=false`, a missing bundled template uses create-only semantics, an existing exact target proceeds without a write, and an existing template missing the exact target fails. Keep bundled-template preflight serial when separate processes add indices because concurrent read and update cycles are last-write-wins.

Pipeline and template preflight errors abort before bulk ingestion starts. These options require an Elasticsearch output.

Use `--batch-size` and `--max-requests` to tune Elasticsearch bulk batches and concurrency. Request-body gzip compression is enabled by default; `--uncompressed` disables it. Authentication flags for direct HTTP(S) Elasticsearch outputs are `--apikey`, `--username`, `--password`, and `--insecure`.

## Required Clarification

Before running an Elasticsearch ingestion, require:

1. At least one complete input.
2. An explicit Elasticsearch index name.
3. An unambiguous Elasticsearch URL, known host, or CLI context.

Ask a short follow-up when any required value is missing. For example: "Which Elasticsearch index should I load that into?" A local file, `file://` URI, or `-` output is complete without an index.

## Command Mapping

Examples:

- `espipe accounts.csv records:customers`
- `espipe users.csv https://host:9200/users`
- `espipe --action upsert --generate-id=true 'docs/**/*.md' env:/documents`
- `espipe 'knowledge/**/*.md' env:/knowledge --template _okf --content markdown`
- `espipe --split /hits response.json output.ndjson`

Use only flags the user requests or that are required to express the destination. Do not reinterpret `--action index` as an overwrite-by-source-ID option; IDs are used only when explicit or generated according to the rules above.

## Execution Checklist

1. Resolve every input path, glob, `file://` URI, HTTP(S) URI, or stdin source.
2. Resolve the final output and require an index for Elasticsearch.
3. Verify direct local files exist; preserve quoted glob patterns for espipe to expand.
4. Add only explicitly requested action, split, ID, safety, content, auth, tuning, pipeline, or template flags.
5. Run `espipe <options> <input>... <output>`.
6. Report the exact command, documents written, and any warnings or errors.

Completion means the command ran with every required input and destination resolved, and the result was reported accurately.
