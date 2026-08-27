## Why

OKF documents currently rely on Elasticsearch dynamic mapping or a user-maintained template, which produces wasteful `text` plus `keyword` mappings for metadata and leaves official OKF fields inconsistently typed. A bundled OKF template gives every `espipe` binary a versioned, ready-to-use mapping without requiring a separate template file.

## What Changes

- Ship a composable Elasticsearch index template for the official OKF v0.2 frontmatter fields and the document shape emitted by `espipe` Markdown ingestion.
- Give official metadata explicit field mappings, including nested provenance, trust, lifecycle, and attested-computation fields.
- Add dynamic templates that map undeclared strings once according to their role instead of creating a `text` field with a `keyword` multifield for every string.
- Extend `--template` so values beginning with `_` select a bundled template, starting with `--template _okf`; filesystem paths keep their existing behavior.
- Give each bundled template a default Elasticsearch template name. `_okf` defaults to `open-knowledge-format`.
- Allow `--template-name` to override the Elasticsearch name for `_okf` and future bundled templates.
- Before each bundled-template ingestion, read the selected Elasticsearch template name. Create it with the target index when absent, or append the target index and update the existing template when the exact index name is not already listed.
- Embed template assets into the executable with `rust-embed` so crates, release archives, and platform packages all contain the same template.

## Capabilities

### New Capabilities

- `okf-index-template`: Defines the bundled OKF template, its supported OKF specification version, explicit mappings, and default dynamic mapping policy.

### Modified Capabilities

- `elasticsearch-index-template`: Allow bundled template selectors, default and overridden installation names, and shared pattern maintenance across target indices alongside existing file-backed templates.

## Impact

- Affected CLI: `--template` accepts a bundled selector such as `_okf` in addition to a path.
- Affected Elasticsearch preflight: bundled template resolution gains an embedded source, a default or overridden installation name, and a read, merge, and conditional update flow.
- Affected cluster permissions: `_okf` requires permission to read the existing composable index template as well as create or update it.
- Affected packaging: `Cargo.toml` gains `rust-embed`, template assets become package inputs, and release builds must prove the asset is available without source-tree files.
- Affected tests and docs: mapping assertions, bundled selector errors, default and overridden template names, shared-template creation and update behavior, binary embedding, CLI help, README examples, and the `espipe` skill.
- External contract: mappings track the official Open Knowledge Format v0.2 specification; future OKF revisions require an intentional template update.
