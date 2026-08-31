## 1. Dependency and target model

- [x] 1.1 Add `elasticrc` 0.1.0 with the local development path `../../../esdiag/cli-extension/esdiag/crates/elasticrc`, refresh the lockfile, and raise espipe's declared Rust version from 1.88 to 1.89.
- [x] 1.2 Add an internal context-output target type that separates the leading-dot Elastic CLI context reference from its required index.
- [x] 1.3 Add focused parser tests for active and named contexts, dotted context names, both Elasticsearch aliases, missing indices, authority syntax, and unsupported applications.

## 2. Config and output resolution

- [x] 2.1 Detect context outputs before existing file and known-host fallbacks while leaving all other output forms unchanged.
- [x] 2.2 Load Elastic CLI config through `ConfigFile::load_with_options` and resolve Elasticsearch from either `current_context` or the named context.
- [x] 2.3 Convert resolved API-key, basic, and unauthenticated values into espipe authentication without including secret values in logs, displays, or errors.
- [x] 2.4 Preserve explicit command-line authentication precedence, apply `--insecure`, append the index to the resolved service base path, and route the result through the existing Elasticsearch output constructor.
- [x] 2.5 Map malformed targets and config, context, service, URL, and resolver failures to actionable startup errors.

## 3. Behavioral verification

- [x] 3.1 Add config fixtures and serialized environment tests for `ELASTIC_CLI_CONFIG_FILE`, home discovery, current and named context selection, missing config data, and failed resolvers.
- [x] 3.2 Add tests for context API-key, basic, no-auth, and explicit-auth override behavior against an HTTP test server, including the final bulk request path.
- [x] 3.3 Verify that resolving a context leaves its config bytes unchanged, skips resolver expressions on unselected services, and never renders resolved secrets.
- [x] 3.4 Run formatting, the full test suite, lint checks used by the repository, and package inspection with the local dependency. Record registry-dependent package verification as a release prerequisite until `elasticrc` 0.1.0 is published.
- [x] 3.5 Start summary timing after input and output initialization, and add a CLI regression test proving credential authorization wait time is excluded.

## 4. User documentation and release notes

- [x] 4.1 Document `.app:/index` and `.context.app:/index`, supported aliases, config discovery order, resolver trust, authentication precedence, examples, and troubleshooting in the README and CLI help.
- [x] 4.2 Add an Unreleased changelog entry for read-only Elastic CLI context output support and the Rust-version change.
- [x] 4.3 After `elasticrc` 0.1.0 is published, verify espipe packaging resolves the registry dependency before release.
