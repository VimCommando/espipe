## Context

`TemplateConfig` currently stores a `PathBuf`, and preflight reads, parses, names, checks, and installs that file before input access. File-backed templates own their `index_patterns`; a mismatch only warns. The new selector must share the parsing and installation path without weakening those rules.

Markdown frontmatter is emitted under `content`, alongside the configured body field. Local file identity is emitted under `origin`. OKF v0.2 defines core concept fields, provenance and trust structures, lifecycle fields, and Attested Computation fields, but it permits producer-defined extensions and two shapes for `verified`.

## Goals / Non-Goals

**Goals:**

- Keep one parsed-template representation and one Elasticsearch installation path for embedded and file-backed sources.
- Make the template asset readable and reviewable as ordinary YAML in the repository.
- Preserve structured associations in repeated OKF objects and avoid automatic dual string mappings.
- Maintain one Elasticsearch template per selected installation name across every index loaded with the same bundled template invocation.

**Non-Goals:**

- Validate whether input documents conform to OKF.
- Infer that Markdown input is OKF or select `_okf` automatically.
- Rewrite file-backed template patterns.
- Support aliases for bundled selectors or remote template registries.
- Promise automatic compatibility with OKF versions after v0.2.

## Decisions

### Resolve `--template` into an explicit source type

Replace the path-only internal value with a source enum containing `File(PathBuf)` and `Bundled(String)`. Classify the raw CLI value by its first character: a leading `_` means bundled, all other values mean file. This keeps the public CLI to one option and leaves relative paths such as `templates/_okf.json` unambiguous.

Resolution returns bytes plus source metadata. Both sources then use the existing config parser and produce the same `ParsedTemplate`. File extensions continue to select JSON, JSON5, or YAML parsing. Bundled assets use their known YAML format rather than pretending to have a user path.

The alternative was a separate `--bundled-template` option. That makes the source explicit but gives users two mutually exclusive ways to perform one operation and does not match the requested `_okf` convention.

### Embed an assets directory with `rust-embed`

Add `rust-embed` and derive one asset collection for `assets/templates/`. Store `_okf.yaml` there and add the asset directory to the Cargo package include list. Each asset declares its default Elasticsearch template name in `_meta.espipe.default_template_name`. The lookup layer exposes logical selector names without extensions, so `_okf` resolves `_okf.yaml` and `open-knowledge-format`, while error reporting can enumerate the embedded catalog.

This follows `esdiag`'s asset model and, unlike `include_str!` per template, gives future bundled templates one catalog and one lookup path. Compression is unnecessary for the first small JSON asset; it can be enabled later without changing behavior.

### Resolve a selected Elasticsearch template name

Read the bundled asset's default installation name, then replace it when the user supplies `--template-name`. `_okf` declares `open-knowledge-format`. Future assets follow the same rule without selector-specific code. Validate the selected name with the existing template-name checks before sending a request.

Preserve `--template-overwrite=false`: it creates a missing selected template with create-only semantics, accepts an existing template that already lists the target, and fails when adding the target would require an update. File-backed templates keep both controls unchanged.

During bundled-template preflight, send `GET /_index_template/{selected_name}` before the template write:

1. On `404`, parse the bundled asset, append the exact target index to its initially empty `index_patterns`, and create `{selected_name}`. Use `PUT` by default or the existing create-only request when overwrite is disabled.
2. On `200`, extract the one exact-name `index_template` body from Elasticsearch's `index_templates` response. Require an array of string `index_patterns`.
3. If the exact target index string is absent and overwrite is enabled, append it without sorting or deduplicating other entries, then `PUT` the full stored template body back to `{selected_name}`. If overwrite is disabled, fail without writing.
4. If the exact string is present, skip the `PUT`. Preflight has already proved the template exists and covers the explicit target entry.

Exact membership, rather than wildcard matching, makes the merge deterministic and produces an audit-friendly list of indices loaded through `_okf`. Preserve the stored body rather than rebuilding it from the embedded asset. This retains mappings, settings, aliases, priority, version, `_meta`, and `composed_of`, including cluster-side edits. The trade-off is that installing a newer `espipe` does not upgrade an existing template's mappings. Mapping upgrades need their own versioned migration policy.

Treat authentication failures, transport errors, unexpected statuses, ambiguous response entries, and invalid stored patterns as fatal preflight errors. Replacing an unreadable stored template with the bundled default could erase cluster configuration, so the safe response is to stop.

File-backed templates do not perform a lookup or merge. Their names still come from the file stem, and mismatched patterns still warn.

The alternative was one template per index. That avoids a read before installation but leaves many identical templates and makes a mapping revision harder to manage. Forcing bundled templates to use only their default names was also considered, but it would make future assets less reusable across teams and environments. Rebuilding the shared template from the bundled asset on every run would silently discard cluster-side edits.

### Map OKF v0.2 according to query role

The asset records `okf_version: "0.2"` and an integer template revision in `_meta`. Explicit mappings cover:

- text search: `content.title`, `content.description`, `content.body`, `content.markdown`, and `content.sources.title`;
- exact filtering and identity: the remaining string-valued official fields, string arrays, and `origin` fields;
- typed values: OKF timestamps as `date`, `sources.usage_count` as `long`, and `parameters.required` as `boolean`;
- repeated structures: `sources`, `verified`, and `parameters` as `nested`, with their children explicitly mapped.

Elasticsearch accepts a single object or an array for an object mapping, so the OKF shorthand form of `verified` remains compatible with a `nested` mapping. Ordinary objects such as `generated`, `usage_window`, `executor`, and `attester` use `object` properties.

Set `date_detection` to `false`. A final dynamic template maps any undeclared string to one `keyword` field with a finite `ignore_above` limit. Producer extensions stay filterable without doubling every string. The explicitly mapped prose fields take precedence. Users who choose a custom body field can provide a file-backed template or add a future explicit bundled mapping; silently treating every unknown string as prose would recreate the mapping growth this change is meant to stop.

The alternative was `dynamic: false`. That avoids mapping growth but makes producer-defined OKF metadata unqueryable. Mapping unknown strings as `text` would favor full-text search at the cost of aggregations and exact filtering, which is the less common role for frontmatter extensions.

### Test the asset at three boundaries

Unit tests decode the embedded asset and assert its complete field mapping, default installation name, and dynamic template. Preflight tests cover default and overridden names, missing and existing template lookups, conditional updates, exact membership, malformed responses, and unchanged file behavior. A distribution-style test runs the compiled binary with a working directory outside the repository and inspects the template requests, proving runtime lookup does not touch the asset directory.

## Risks / Trade-offs

- [OKF changes after v0.2] The mappings can become stale. Store the supported spec version and template revision in `_meta`, document the pin, and update the asset through a reviewed change.
- [A producer extension needs full-text search] The string fallback maps it as `keyword`. Keep known prose explicit and document file-backed templates as the escape hatch.
- [Very long URI or path values exceed the keyword limit] Values remain in `_source` but may not be indexed. Choose and test a limit suitable for expected OKF paths and URIs, and state it in the asset comments or documentation.
- [Concurrent runs can lose one appended index] Two processes may read the same pattern list and race to update it. Elasticsearch's index template API does not expose an atomic append, so document this last-write-wins limit instead of promising unsafe retry logic.
- [Existing templates do not receive mapping upgrades] Preserving the stored template protects cluster-side edits but leaves its original mapping revision in place. Handle mapping upgrades in a separate versioned migration design.
- [Template lookup needs another cluster privilege] `_okf` now reads the existing template before any write. Document the required Elasticsearch template read and management privileges and return the lookup response details on authorization failure.
- [An override can name an unrelated existing template] The merge preserves that template and appends the target index, which can broaden where its mappings apply. Treat the explicit override as user intent and document that it should name a template dedicated to the selected bundled asset.
- [Nested mappings cost more than plain objects] OKF's repeated source, verifier, and parameter records need per-entry association for correct queries. The extra hidden documents are bounded by those metadata lists.

## Migration Plan

This is additive. Existing file-backed commands and runs without `--template` keep their behavior. Release packaging adds the asset directory and dependency in the same change; rollback removes selector support and the embedded asset without changing any indexed document. Templates created under default or overridden names remain cluster resources and can be removed by an administrator if desired.
