## Why

`espipe` can use direct URLs, its own known-host file, or Elastic CLI environment variables, but it cannot address a cluster stored in a user's `.elasticrc`. Users must copy the URL and credentials into another configuration source instead of selecting an existing Elastic CLI context.

## What Changes

- Add an Elasticsearch output form based on esdiag's leading-dot context references: `.app:/index` selects the active context and `.context.app:/index` selects a named context.
- Limit the application segment to Elasticsearch aliases supported by `elasticrc`, since an espipe output must address the Elasticsearch bulk API.
- Discover and load Elastic CLI config through the read-only `elasticrc` crate, including `ELASTIC_CLI_CONFIG_FILE` and supported files in the user's home directory.
- Resolve the selected context's Elasticsearch URL and authentication only when a context output is requested. Explicit command-line authentication remains higher precedence than context authentication.
- Report document processing time without counting `.elasticrc` resolution or time spent waiting for credential authorization.
- Report invalid references, missing config, missing contexts or Elasticsearch services, resolver failures, and invalid index paths as startup errors without writing to `.elasticrc`.
- Use the local `elasticrc` crate path during development while declaring its publishable version for packaged builds. Raise espipe's minimum Rust version from 1.88 to 1.89.
- Document the new output form, config discovery, authentication precedence, and read-only behavior.

## Capabilities

### New Capabilities

- `elastic-cli-context-output`: Resolve active or named Elastic CLI contexts into authenticated Elasticsearch outputs with an explicit target index.

### Modified Capabilities

None.

## Impact

- Affected CLI surface: the final positional output accepts `.app:/index` and `.context.app:/index`.
- Affected code: output target parsing, Elasticsearch client construction, authentication selection, tests, and user documentation.
- New dependency: `elasticrc` 0.1.x, initially sourced from `/Users/reno/Development/worktrees/esdiag/cli-extension/esdiag/crates/elasticrc` with a registry version for packaging.
- Toolchain: raise espipe's declared minimum Rust version from 1.88 to 1.89 to match `elasticrc`.
- Security: context secrets may come from inline values or supported `elasticrc` resolvers. espipe exposes a resolved secret only when constructing the Elasticsearch client and must not log it.
