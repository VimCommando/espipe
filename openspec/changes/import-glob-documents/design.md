## Context

`espipe` currently accepts one input URI followed by one output URI and maps local file extensions to CSV or line-delimited JSON readers. The ingest loop consumes one raw JSON document at a time through `Input::read_line`, then forwards each document to the selected output. File imports need to fit that document stream model while producing JSON object documents according to each file's format.

Markdown collections are a primary target for this change. Markdown frontmatter should become `content.*` document fields, and the Markdown body should remain available as searchable content under a configurable `content.<field>` name.

## Goals / Non-Goals

**Goals:**

- Accept one or more local file inputs, including file lists produced by shell glob expansion.
- Accept glob patterns as local input values, including recursive patterns such as `**/*.md`, when the shell does not expand them first.
- Produce JSON object documents from matched files using extension-specific parsing.
- Parse Markdown YAML frontmatter into `content.*` document fields.
- Handle common file extensions explicitly: `.md`, `.markdown`, `.txt`, `.text`, `.log`, `.yml`, `.yaml`, `.json`, `.ndjson`, and `.jsonl`.
- Store file content in the `content.<field>` field named by `--content`, defaulting to `content.body`.
- Keep existing CSV, NDJSON, JSON, stdin, and HTTPS input behavior unchanged.
- Preserve the raw-document output path by serializing each file document once into `Box<RawValue>`.

**Non-Goals:**

- Remote glob expansion over HTTPS or multiple remote inputs.
- Custom frontmatter delimiters beyond the standard `---` YAML block at the start of Markdown files.
- JSON array unwrapping or JSON-path splitting; a future `--split <json_path>` option can cover that separately.
- Automatic extraction of metadata such as extension or modified time unless a later change specifies those fields.

## Decisions

Change CLI parsing so the final positional argument remains the output URI and every preceding positional argument is treated as an input. The existing two-positional form remains valid. A single input positional continues to use existing CSV, JSON, NDJSON, stdin, and HTTPS behavior when it matches those modes. Multiple local input positionals select file-document mode and are treated as plain-text files or glob patterns.

Use file-document input mode when multiple local input files are provided, when a local input contains glob metacharacters, or when a single local plain-text file such as `.md` or `.txt` is provided. This resolves shell-expanded globs naturally: the shell can pass many concrete files, and `espipe` imports each one as a document. Rust-side glob expansion remains necessary for quoted recursive patterns and shells that do not expand `**`.

Prefer shell expansion when users already rely on it; it avoids adding traversal work to `espipe` and lets the shell provide concrete paths. Keep Rust-side expansion for quoted patterns because it avoids shell-specific behavior, can support consistent `**` semantics, and can produce better no-match errors. Performance should be dominated by file reads and JSON serialization for normal documentation collections; for very large trees, Rust-side expansion avoids command-line argument length limits while shell expansion avoids one dependency and one discovery pass inside `espipe`.

Expand any remaining glob patterns before ingestion, merge them with concrete file inputs, de-duplicate exact path matches, and sort matched file paths lexicographically. Deterministic ordering makes stdout/file output reproducible and keeps tests stable even when the shell supplied paths in filesystem order. Directories and non-files will be ignored; a pattern or input set that produces no regular files will fail before output initialization.

Add `--content <field_name>` to the top-level CLI and pass the value into input construction. The default is `body`, producing `content.body`. The field name must be non-empty and is interpreted as a subfield under `content`; for example `--content markdown` produces `content.markdown`. It cannot contain dots because `content.<field_name>` is responsible for namespacing.

Represent frontmatter and YAML mapping fields under the `content` object as `content.<metadata_field>`, such as `content.title` or `content.summary`. Reject content-field conflicts when Markdown frontmatter or YAML metadata uses the same field name as the configured content subfield. This keeps file-derived metadata grouped with text content and avoids top-level collisions with future `file.*` metadata.

When file-document input resolves more than one regular file after glob expansion, shell-expanded argument handling, filtering, and de-duplication, add `file.path` and `file.name` to each emitted document. `file.path` is the matched local path string used for ordering, and `file.name` is the final path component. Do not add `file.path` or `file.name` for a single direct file-document input.

Represent file-document imports as a new `Input` variant that owns the matched path list and an index cursor. Each `read_line` call reads the next file, constructs a `serde_json::Map`, serializes it to a string, and converts it to `Box<RawValue>`. This preserves the existing output interface and avoids forcing output implementations to understand file documents.

For Markdown files, parse a YAML frontmatter block only when the file starts with `---` followed by a closing `---` delimiter on its own line. YAML mappings become `content.*` document fields. The content after the closing delimiter becomes the configured `content.<field>` field. Markdown without frontmatter imports with only the configured content field, plus conditional `file.*` fields for multi-file imports.

For `.txt`, `.text`, `.log`, and unknown UTF-8 extensions, import the full file contents into the configured `content.<field>` field. This makes file imports useful for text collections while keeping format-specific metadata extraction limited to formats with clear object semantics.

For `.yml` and `.yaml`, parse the file as one YAML document and convert mapping fields into the `content` object. The YAML root must be a mapping; scalar, sequence, and null roots are rejected because Elasticsearch documents need object shape.

For `.ndjson` and `.jsonl`, keep line-delimited streaming behavior. Each non-empty line must parse as a JSON object, and each line emits one document. These extensions are explicit signals that the input is already document-delimited.

For `.json`, use whole-file JSON object handling. Read the file as UTF-8, parse the full file as JSON, and emit one document if the root is an object. JSON arrays are rejected rather than split; array unwrapping is out of scope for this change and can be handled by preprocessing or a future `--split <json_path>` option. `.json` files do not fall back to line-delimited parsing; users with line-delimited JSON should use `.ndjson` or `.jsonl`.

Reject binary files rather than base64-encoding or lossy-decoding them. Implement detection by reading file bytes and requiring valid UTF-8 text before document construction. If a file cannot be decoded as UTF-8, fail with an error that identifies the path. Warnings and user-facing errors for file import discovery and parsing should be written to stderr.

Use existing `serde_yaml` support for YAML parsing and add a glob-walking dependency only if the standard library cannot provide robust recursive pattern expansion. Candidate crates should preserve platform path behavior and support `**` recursion.

## Risks / Trade-offs

- Large matched files require full-file reads before a document can be emitted -> keep this scoped to one-document-per-file imports and surface file read errors with the path that failed.
- Shell-expanded file lists can exceed OS command-line argument limits for very large trees -> support quoted glob patterns so Rust-side expansion can handle those cases.
- YAML frontmatter can contain non-object values -> require the frontmatter root to be a mapping so document fields are predictable.
- Frontmatter fields can produce unsupported JSON values or duplicate the content field -> convert YAML through serde into JSON-compatible values and reject content-field conflicts explicitly.
- `.json` whole-file parsing reads the file into memory -> acceptable for one-document file imports; users with large document streams should use `.ndjson` or `.jsonl`.
- Different shells expand globs differently -> accept already-expanded file lists and also support quoted patterns for consistent Rust-side `**` behavior.
- Glob ordering can differ by filesystem traversal -> sort normalized matched paths before ingestion.

## Migration Plan

No migration is required. Existing inputs without glob metacharacters continue through the current CSV, JSON, NDJSON, stdin, and HTTPS paths. Rollback is limited to removing the new CLI option, dependency, and `Input` variant before release if needed.
