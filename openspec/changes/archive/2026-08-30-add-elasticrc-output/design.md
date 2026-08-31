## Context

The final positional argument is parsed as `UriRef<String>` and passed to `Output::try_new`. That constructor currently distinguishes environment, direct HTTP, file, stdout, and known-host outputs. A leading-dot context reference has no URI scheme, so the current code treats it as a file path.

The published `elasticrc` 0.1.0 crate owns Elastic CLI config discovery, JSON and YAML parsing, service selection, resolver execution, URL validation, and redacted secret values. Its `ContextServiceReference` parses `.service` and `.context.service` by treating the rightmost dot-separated segment as the service. Selecting a context and calling `ServiceConfig<Elasticsearch>::resolve` evaluates expressions only for that service.

The crate declares Rust 1.89, one minor version above espipe's previous minimum.

## Goals / Non-Goals

### Goals

- Add context target parsing before the existing file and known-host fallbacks.
- Reuse `elasticrc` as the sole implementation of Elastic CLI config and credential resolution.
- Feed the resolved URL and authentication into the existing Elasticsearch output path so bulk actions, compression, pipelines, templates, batching, and summaries remain unchanged.
- Keep explicit command-line authentication at the top of the precedence order.

### Non-goals

- Writing or migrating Elastic CLI config.
- Supporting Kibana or Elastic Cloud management services as bulk outputs.
- Replacing `env:/index`, direct URLs, or `~/.espipe/hosts.yml`.
- Adding a command-line option for an Elastic CLI config path. `ELASTIC_CLI_CONFIG_FILE` already supplies this override.
- Reimplementing secret resolvers in espipe.

## Decisions

### Parse the context reference separately from the index

Add an output-target value that holds an `elasticrc::ContextServiceReference` and an index. Parse the original final positional string as `<reference>:/<index>`, then pass only `<reference>` to the crate parser. Require a leading dot, exactly one slash after the colon, a non-empty index, and no URI authority.

Run this parser before `Output::validate_environment_target`, generic file handling, and known-host lookup. Values without a recognizable context prefix continue through the existing output rules. Values that clearly attempt the context form but use an unsupported application or malformed index return a targeted error instead of becoming surprising file paths.

Using the `elasticrc` reference parser keeps dotted context names and service aliases aligned with esdiag. Teaching `fluent-uri` a new scheme would not work because URI schemes cannot begin with a dot. Treating the entire value as a path would leave output dispatch ambiguous.

### Resolve only Elasticsearch from the selected config context

Load config with `ConfigFile::load_with_options(None, None)`. Select the named context with `ConfigFile::context`, or use `current_context_name` for an active reference. Read the context's Elasticsearch `ServiceConfig` and call `resolve`. Reject a parsed Kibana or Cloud reference before config resolution.

This delegates discovery order, schema handling, URL checks, resolver limits, platform credential stores, and redaction to the shared crate. Copying esdiag's config structs or resolver code would create a second compatibility boundary and could handle secrets differently.

### Convert the resolved service at the output boundary

Convert `elasticrc::ResolvedAuth` into espipe's `Auth` only inside output construction. Call `expose_secret()` at that point and move the resulting value directly into the Elasticsearch client builder. Do not add resolved values to errors, display implementations, or logs.

If `Auth::try_new` produced an explicit API key or basic credentials, retain it. Use context authentication only when the explicit value is `Auth::None`. This matches `env:/index` precedence and lets a user recover from a stale context credential while still reusing its URL. `--insecure` continues to control certificate validation because the current Elastic CLI service schema has no corresponding TLS field.

The resolved service URL joins the requested index in the same way as `env:/index`: preserve any configured base path, append the index, and clear query and fragment components. Then call the existing Elasticsearch output constructor. A separate output implementation would duplicate bulk and preflight behavior.

### Use the published registry dependency

Development started with `elasticrc` declared as both version `0.1.0` and the relative local path `../../../esdiag/cli-extension/esdiag/crates/elasticrc`. Once 0.1.0 was published, remove the path and compile against the registry package. Run `cargo package` to prove a clean packaged build resolves the published crate.

Raise espipe's declared `rust-version` from 1.88 to 1.89. Keeping 1.88 while depending on a crate that declares 1.89 would misstate the supported toolchain.

### Cover parsing, discovery, authentication, and transport boundaries

Unit tests should exercise active and named references, dotted context names, both Elasticsearch aliases, malformed index forms, and unsupported applications. Config fixtures should cover discovery through `ELASTIC_CLI_CONFIG_FILE`, current-context selection, named-context selection, API-key and basic authentication, no authentication, missing config data, failed resolvers, and an unselected Kibana resolver.

An HTTP test server should verify the final bulk path and authorization header. Tests that change process environment must serialize those changes. A file snapshot or byte comparison should prove config resolution does not mutate the file.

### Start summary timing when document processing starts

Create the runtime summary timer after both input and output initialization complete, immediately before the document read and send loop. This makes the printed duration describe document processing through output close. It excludes config discovery, credential resolver work, and user authorization waits.

Trying to subtract only authorization time would require resolver-specific timing from `elasticrc` and would still leave the summary dependent on output setup details. A processing timer has one clear boundary and matches the wording of the summary.

## Risks / Trade-offs

- [A context name or local filename resembles the new syntax] -> Reserve only a leading-dot value that attempts the `:/` context form. Explicit local paths can retain a filesystem prefix such as `./`.
- [Resolver-backed credentials can execute trusted local programs] -> Rely on `elasticrc`'s direct process execution, shell rejection, time limit, output limit, and credential-environment filtering. Document that users must trust their config.
- [Explicit authentication still requires resolving the selected service block] -> Document precedence as request authentication precedence, not as a promise to skip config validation or resolver evaluation.
- [The local crate and published crate diverge] -> Pin a compatible 0.1.x version, run package verification, and remove the path only when normal registry consumption is ready.
- [Rust 1.89 narrows compatibility] -> Declare the new minimum accurately and include it in release notes.
- [Startup work no longer appears in the runtime summary] -> Define the value as processing time and test the boundary with a delayed credential resolver.

## Migration Plan

1. Add the local path plus registry version dependency and set `rust-version` to 1.89.
2. Add parsing and resolution without changing existing output branches.
3. Verify unit, integration, packaging, and documentation checks with the local crate.
4. Confirm the declared `elasticrc` version is published before publishing espipe.

Rollback removes the context-output branch and dependency. Existing output forms and config files require no migration.
