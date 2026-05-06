## Context

`espipe` currently chooses input behavior from URI scheme and path extension. CSV rows are converted to JSON objects, NDJSON lines are validated as JSON objects, and both feed outputs as owned `Box<serde_json::value::RawValue>`. Remote HTTPS inputs are fetched into a temporary file before the existing local readers consume them.

Toon input should preserve those properties: extension-driven selection, object-document validation, raw JSON output compatibility, and bounded memory use for large inputs. The release uses the crates.io `toon-format` parser with default features disabled so `espipe` does not pull in the parser crate's CLI/TUI dependency graph.

## Goals / Non-Goals

**Goals:**

- Recognize `.toon` local and HTTPS input resources.
- Stream Toon documents from a reader instead of requiring a whole input buffer.
- Convert each Toon document to a JSON object `RawValue` before output dispatch.
- Use the crates.io `toon-format` parser dependency with default features disabled.
- Cover valid local, valid remote, malformed, and non-object Toon inputs with tests.

**Non-Goals:**

- Add a new CLI flag for selecting input format.
- Add Toon output support.
- Add authenticated remote fetch behavior.
- Treat `.json` files as Toon or auto-detect Toon content without extension or response metadata.

## Decisions

1. Add `InputKind::Toon` and keep format selection extension based.

   `.toon` will map to the new input kind in the same place `.csv`, `.ndjson`, and `.json` are detected. This keeps CLI behavior predictable and avoids content sniffing. The alternative was a `--format toon` flag, but that would be a broader CLI change and is unnecessary for the current path-based model.

2. Implement Toon as a streaming reader variant.

   Local Toon input should open a file and stream document chunks separated by lines whose trimmed content is exactly `---`, decoding each chunk with the Toon parser. Remote Toon input may continue using the current HTTPS temporary-file path, then open the same reader over that file. The alternative was to use blank lines as separators, but the Toon spec treats blank lines as valid inside one root object.

3. Emit JSON object documents as `Box<RawValue>`.

   Each decoded Toon document will be validated as an object, serialized once to a JSON object string, and converted to `RawValue`. This matches CSV, YAML, and file-document conversion points while preserving the raw output contract after parsing. The alternative was to pass `serde_json::Value` through the output layer, but that conflicts with the current RawValue pipeline.

4. Treat remote Toon metadata as explicit support.

   Remote detection should accept `.toon` URLs and Toon-specific content types such as `application/toon` and `text/toon`, while including those types in the request `Accept` header. Ambiguous `application/json` responses remain JSON/NDJSON rather than Toon.

## Risks / Trade-offs

- Dependency feature bloat -> Disable `toon-format` default features so the release does not include the parser crate's CLI/TUI dependencies.
- Decode API scope -> `toon-format` exposes string-based decode helpers, so `espipe` owns the outer multi-document reader and uses the parser to decode each document chunk.
- Toon document shape ambiguity -> Require each emitted document to be a JSON object and reject arrays/scalars with a clear error.
- Remote fetch still stages to disk -> This preserves existing remote-input architecture while allowing parsing to stream from the staged file; direct response-body streaming can be a future optimization.
- RawValue conversion still serializes decoded Toon values once -> This is necessary because outputs consume JSON bytes; tests should ensure the steady-state output path remains `RawValue`.
