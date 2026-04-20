---
name: espipe
description: Use when the user wants to load, import, ingest, pipe, or bulk-send local or remote CSV, NDJSON, or JSON data into Elasticsearch with espipe, especially from plain-English prompts that mention a file, cluster, host, or index.
---

# Espipe Ingestion

Translate the user's ingestion request into an `espipe` command and run it when the destination is clear.

## Inputs

Supported inputs:

- Local `.csv`, `.ndjson`, and `.json` files
- `file://` URIs for local files
- `https://` URLs for unauthenticated remote `.csv`, `.ndjson`, and `.json` sources
- `-` for NDJSON on `stdin`

For CSV input, assume the first row is a header row. CSV values stay strings.

## Outputs

Use these Elasticsearch target forms:

- `http://host:9200/index-name`
- `https://host:9200/index-name`
- `known-host:index-name`

Known hosts come from `$ESPIPE_HOSTS` or `~/.espipe/hosts.yml`.

If the user says something like "my `records` cluster" and `records` is a host nickname, target `records:index-name`.

Do not read the user's `hosts.yml` unless explicitly asked and granted permission.

## Required Clarification

Do not run `espipe` until the index name is explicit.

Ask a short follow-up when the user provides a file and cluster or host but no index, for example:

- "Which Elasticsearch index should I load that into?"

Also ask when the destination cluster or host is missing or ambiguous.

## Command Mapping

Default to the `create` bulk action unless the user explicitly asks to overwrite or upsert.

Examples:

- "Load my `accounts.csv` file into my `records` cluster's `customers` index"
  Run: `espipe accounts.csv records:customers`
- "Import `users.csv` into `http://localhost:9200/users`"
  Run: `espipe users.csv http://localhost:9200/users`
- "Send `docs.ndjson` to `orders`"
  Ask which cluster or URL should receive the `orders` index.
- "Load `accounts.csv` into my `records` cluster"
  Ask which index on `records` should receive the data.

Use `--action index` only when the user explicitly asks to overwrite existing IDs or use the Elasticsearch `index` bulk action. Use `--action update` only when the user explicitly asks for updates and the source documents include `_id`.

## Execution Checklist

1. Resolve the input path or URI from the user's prompt.
2. Resolve the destination as a full Elasticsearch target, including the index name.
3. If the index name is missing, ask for it before doing anything else.
4. Verify local input files exist before running the command.
5. Run `espipe <input> <output>` with any explicitly requested auth or action flags.
6. Report the exact command used and the ingestion result or failure.

## Notes

- Prefer `known-host:index-name` when the user refers to a named cluster already configured on the machine.
- Do not invent host aliases, URLs, credentials, or index names.
- `http://` and `https://` are valid for Elasticsearch outputs. Remote inputs may also use `http://` or `https://` when they point to supported `.csv`, `.ndjson`, or `.json` sources.
