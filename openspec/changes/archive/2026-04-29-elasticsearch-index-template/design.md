## Context

`espipe` currently constructs an output before reading input documents, then forwards raw documents to the output. Elasticsearch output starts a bounded bulk worker and sends `_bulk` requests after documents arrive. Users who need mappings or settings must currently create index templates outside `espipe`, which makes one-shot imports less reproducible.

The requested `--template <path>` option should install an Elasticsearch index template before any document batch is sent. If the cluster rejects the template, ingestion must not start sending documents.

## Goals / Non-Goals

**Goals:**

- Add `--template <path>` for Elasticsearch outputs.
- Add `--template-name <name>` as an optional name override.
- Add `--template-overwrite <bool>` with a default of `true`.
- Read and validate `.json`, `.jsonc`, and `.json5` template files before output sends any documents.
- Send the template to Elasticsearch before the first bulk request.
- Warn when the template's `index_patterns` do not match the target index name from the output URI.
- Abort the run if CLI arguments are invalid, the selected output is incompatible, the template file cannot be read, the template file is invalid, or Elasticsearch rejects the template request.
- Keep runs without `--template` unchanged.

**Non-Goals:**

- Supporting component templates as a separate CLI option.
- Supporting legacy `/_template` APIs.
- Preserving comments or original formatting when sending the template body.
- Applying templates to file or stdout outputs.
- Retrying rejected template requests.

## Decisions

Pass optional template configuration from `main.rs` into `Output::try_new`: template path, optional template name override, and overwrite boolean. `--template-name` and `--template-overwrite` are only meaningful with `--template`; reject either option when `--template` is absent. `Output::try_new` will reject template-related options for non-Elasticsearch outputs so accidental local file/stdout runs fail before ingestion. These argument and output compatibility checks must run before opening or reading input content.

Read the template file during output construction, before opening or reading input content. Parse `.json` files with strict JSON semantics. Parse `.jsonc` and `.json5` files with `serde_json5` or equivalent tolerant JSON-with-comments support so users can include Elasticsearch-supported C-style block comments, triple-quote syntax, and common JSON5-style authoring conveniences. Convert the parsed result into `serde_json::Value` for validation, `index_patterns` inspection, and request serialization. File read and parse errors should include the path and be written to stderr.

Use the Elasticsearch composable index template API only. When overwriting is enabled, send `PUT /_index_template/{template_name}`. When `--template-overwrite=false`, send `POST /_index_template/{template_name}?create=true` so Elasticsearch treats an existing template as a fatal create-only conflict. Do not support the legacy `/_template/{name}` API. When create-only mode reports that the template already exists, treat that response as fatal and abort before any document batch is sent.

Derive the default `{template_name}` from the template file name without its final extension. For example, `--template ./templates/logs-docs.json` installs template `logs-docs`. If `--template-name <name>` is provided, use that exact name instead. Reject an empty derived or explicit template name.

The template JSON owns `index_patterns`; `espipe` will not synthesize or mutate it. After parsing the JSON, inspect a top-level `index_patterns` string or string array. Evaluate each string using Elasticsearch multi-target syntax for local target-index warning checks: comma-separated expressions are processed left-to-right; wildcard `*` matches zero or more characters; dash-prefixed expressions exclude previous matches; exclusions do not affect later inclusions; a lone `-` expression is invalid. If no expression set matches the target output index name, emit a warning to stderr but continue. If `index_patterns` is missing, has an unexpected shape, or contains invalid multi-target syntax, rely on Elasticsearch validation for acceptance and warn to stderr that the match check could not be performed.

Run the template request before spawning or before feeding the bulk worker. The simplest implementation is to make Elasticsearch output construction async or add an explicit async preflight method that is called before the ingest loop. The important ordering guarantee is that no document can be sent until the template response is successful.

Treat any non-2xx template response as fatal. Include the HTTP status and Elasticsearch error body where available, written to stderr. Do not continue to bulk indexing after a template rejection.

Use `Content-Type: application/json` for the template request. Send the parsed template as normalized JSON rather than the original file bytes; this strips comments and formatting but keeps the accepted template semantics stable across `.json`, `.jsonc`, and `.json5` inputs.

## Risks / Trade-offs

- Filename-derived template names can collide across directories -> allow `--template-name` for explicit naming.
- Installing a composable index template is not the same as creating an index -> document that Elasticsearch applies the template when matching indices are created; users must include appropriate `index_patterns` in the JSON file.
- Updating an existing template can change cluster state -> default overwrite preserves the user's requested behavior, while `--template-overwrite=false` uses Elasticsearch create-only semantics to fail if the template already exists.
- Local index-pattern matching mirrors Elasticsearch multi-target include/exclude ordering for a single target index, but cannot account for alias resolution or cluster state -> warn only; Elasticsearch remains the source of truth.
- Normalizing `.jsonc` and `.json5` input strips comments before sending -> acceptable because comments are authoring aids, not cluster state.
- Full JSON5 syntax may accept input Elasticsearch would not accept as raw JSON-with-comments -> parsing and serializing to strict JSON makes the request body valid JSON before it reaches Elasticsearch.
- Async preflight may require reshaping output construction -> keep the change scoped by adding the preflight at the Elasticsearch output boundary rather than inside input parsing or the bulk worker.

## Migration Plan

No migration is required. Existing invocations without `--template` behave as they do today. Rollback removes the CLI option, template config plumbing, and Elasticsearch preflight request.
