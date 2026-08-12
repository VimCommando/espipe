---
name: espipe
description: Use when the user wants to load, import, ingest, pipe, or bulk-send data or documents with espipe, including local or remote CSV, NDJSON, JSON, and Toon; local text, Markdown, YAML, PDF, Office, RTF, or EPUB files; file globs; and Elasticsearch, file, or stdout destinations.
---

# Espipe Ingestion

Translate the user's ingestion request into an `espipe` command and run it when the input and destination are clear.

## Inputs

Supported inputs include:

- Local `.csv`, `.csv.gz`, `.ndjson`, `.ndjson.gz`, `.json`, and `.toon` files
- Local Markdown, text, YAML, JSON, NDJSON, JSONL, and Toon file documents
- Local PDF, Word, PowerPoint, Excel, OpenDocument, RTF, and EPUB files. These are converted to GitHub-Flavored Markdown; conversion is local-only and image-only/scanned PDFs are not OCR'd.
- `file://` URIs for local files
- `http://` and `https://` URLs for unauthenticated remote `.csv`, `.ndjson`, `.json`, and `.toon` sources
- `-` for NDJSON on `stdin`

For CSV input, assume the first row is a header row. CSV values stay strings.

The user may supply multiple local inputs; the final positional argument is the output. Shell-expanded file lists and quoted recursive globs such as `'docs/**/*.pdf'` are supported. Keep glob patterns quoted in commands so `espipe` performs recursive discovery. Use `--content <field>` when the user requests a file-document content subfield other than the default `content.body`.

Recognized local non-text extensions are `.doc`, `.docx`, `.docm`, `.odt`, `.pdf`, `.ppt`, `.pps`, `.pot`, `.pptx`, `.pptm`, `.ppsx`, `.ppsm`, `.rtf`, `.epub`, `.xls`, `.xlsx`, `.xlsm`, `.xlsb`, `.ods`, and `.odp`.

## Outputs

Elasticsearch targets use these forms:

- `http://host:9200/index-name`
- `https://host:9200/index-name`
- `known-host:index-name`

Known hosts come from `$ESPIPE_HOSTS` or `~/.espipe/hosts.yml`.

If the user says something like "my `records` cluster" and `records` is a host nickname, target `records:index-name`.

Do not read the user's `hosts.yml` unless explicitly asked and granted permission.

Other supported outputs are `-` for stdout, local `.ndjson` or `.ndjson.gz` paths, and corresponding `file://` URIs. File outputs truncate an existing target.

## Required Clarification

For Elasticsearch output, do not run `espipe` until the index name is explicit.

Ask a short follow-up when the user provides a file and cluster or host but no index, for example:

- "Which Elasticsearch index should I load that into?"

Also ask when the Elasticsearch destination cluster or host is missing or ambiguous. An explicit file path or `-` is a complete non-Elasticsearch destination and needs no index name.

## Command Mapping

Default to the `create` bulk action.

Examples:

- "Load my `accounts.csv` file into my `records` cluster's `customers` index"
  Run: `espipe accounts.csv records:customers`
- "Import `users.csv` into `http://localhost:9200/users`"
  Run: `espipe users.csv http://localhost:9200/users`
- "Send `docs.ndjson` to `orders`"
  Ask which cluster or URL should receive the `orders` index.
- "Load `accounts.csv` into my `records` cluster"
  Ask which index on `records` should receive the data.

Use `--action index` only when the user explicitly requests the Elasticsearch `index` bulk action. Do not treat it as an overwrite-by-source-ID option: espipe emits index metadata without an `_id`, so Elasticsearch assigns IDs. Use `--action update` only when the user explicitly requests updates and every source document has a string `_id`; it removes `_id` from the document body and uses it as the update target. espipe does not expose `doc_as_upsert`, so do not claim to perform true upserts.

## Execution Checklist

1. Resolve every input path, glob, `file://` URI, HTTP(S) URI, or stdin request from the user's prompt.
2. Resolve the final output URI. For Elasticsearch, require a full target including the index name; otherwise accept stdout or a supported local NDJSON output.
3. If the Elasticsearch index, host, or cluster is missing, ask before doing anything else.
4. Verify direct local input files exist before running; preserve quoted glob patterns for espipe to resolve.
5. Run `espipe <input>... <output>` with any explicitly requested auth, content, tuning, or action flags.
6. Report the exact command used and the ingestion result or failure.

## Notes

- Prefer `known-host:index-name` when the user refers to a named cluster already configured on the machine.
- Do not invent host aliases, URLs, credentials, or index names.
- `http://` and `https://` are valid for Elasticsearch outputs. Remote inputs may also use either scheme when they point to supported `.csv`, `.ndjson`, `.json`, or `.toon` sources.
