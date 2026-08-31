---
type: Reference
title: Output
description: File, standard output, and Elasticsearch destinations, including bulk actions, tuning, and index templates.
resource: https://github.com/VimCommando/espipe/blob/main/docs/output.md
tags:
  - espipe
  - output
  - elasticsearch
  - bulk-api
status: stable
---

# Output

Elasticsearch connection targets use the credentials and resolvers described
in [Authentication](authentication.md).

## Supported output forms

- `-`
  Writes raw JSON lines to `stdout`.
- `path/to/output.ndjson`
  Writes raw JSON lines to a local file, truncating any existing file.
- `path/to/output.ndjson.gz`
  Writes gzip-compressed raw JSON lines to a local file, truncating any existing file.
- `http://host:9200/index-name`
  Sends documents to Elasticsearch using the `_bulk` API.
- `https://host:9200/index-name`
  Sends documents to Elasticsearch over TLS.
- `known-host:index-name`
  Resolves `known-host` from a local hosts file and sends to the named index.
- `env:/index-name`
  Reads the cluster URL and optional API key from environment variables or `.env`.
- `.es:/index-name` or `.elasticsearch:/index-name`
  Reads the Elasticsearch service from the active Elastic CLI context.
- `.context.es:/index-name` or `.context.elasticsearch:/index-name`
  Reads the Elasticsearch service from a named Elastic CLI context. Context names may contain dots.

## Elasticsearch output

Elasticsearch requests use gzip compression by default. `espipe` retries `429 Too Many Requests` responses with exponential backoff and logs item-level failures from partial bulk responses.

`400 Bad Request` bulk responses are logged and counted as zero successful documents for that batch.

## Bulk actions

`--action` accepts `create`, `index`, `update`, or `upsert`. The default is `index`. Update wraps the source in `{ "doc": ... }`; upsert also adds `"doc_as_upsert": true`.

For `--action update` and `--action upsert`, every input document must:

- be a JSON object
- have an explicit string `_id`, or be a local file document with generated IDs enabled

For every action, a top-level string `_id` becomes the transport ID and is removed from the source. Non-file inputs never receive generated IDs.

## Bulk tuning

Multi-source local imports use 500-document bulk requests by default. Other inputs use 5,000. Override this with `--batch-size`; use `--max-requests` to change the default limit of 16 in-flight requests.

### Index templates

`--template` accepts a JSON, JSONC, JSON5, YAML, or YML composable index template file. A value beginning with `_` selects a template compiled into `espipe` instead. File-backed templates keep their own `index_patterns`; a mismatch with the output index produces a warning.

The bundled `_okf` template maps Open Knowledge Format v0.2 metadata and the `content.body`, `content.markdown`, and `origin` fields emitted by local document ingestion. Official identifiers and categorical metadata use `keyword`, prose uses `text`, timestamps use `date`, and repeated source, verifier, and parameter records use `nested`. Automatic date detection is disabled. Unknown strings become one `keyword` field with `ignore_above: 2048`, without a `text` plus `keyword` multifield.

```bash
espipe 'knowledge/**/*.md' env:/team-knowledge --template _okf --content markdown
```

`_okf` installs as `open-knowledge-format` by default. Use `--template-name team-okf` to select another cluster-side name. On each run, `espipe` reads that template. If it does not exist, `espipe` creates it with the output index in `index_patterns`. If it exists, `espipe` appends the exact output index when absent and writes the stored template body back without replacing its mappings, settings, aliases, priority, version, metadata, or component references. An existing wildcard does not suppress the exact index entry.

`--template-overwrite=false` uses create-only semantics when the selected template is absent. It accepts an existing template only when the exact output index is already listed. Reading and writing bundled templates requires the corresponding Elasticsearch index-template privileges.

Concurrent processes can lose one another's appended index because Elasticsearch has no atomic index-pattern append operation. Run bundled-template preflight serially when separate processes target new indices. An overridden name should identify a template dedicated to that bundled asset; selecting an unrelated template broadens its index coverage.

Local import summaries separate discovered files from documents: `Piped 5,850 of 5,850 docs from 6,246 files ...`. Skipped files count as files, not documents.

## File and stdout output

For file and `stdout` targets, `espipe` writes one raw JSON document per line. It does not emit Elasticsearch bulk action metadata lines for these outputs.
