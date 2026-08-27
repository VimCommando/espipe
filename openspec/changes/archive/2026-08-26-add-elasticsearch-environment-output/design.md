## Context

Output dispatch treats HTTP and HTTPS schemes as direct Elasticsearch targets, `file` as local output, and other schemes as configured host names. The previous environment-backed branch reserved both `es` and `elasticsearch` before configured-host lookup. Environment settings came only from the process environment. See `proposal.md` for the reason for changing that behavior.

The new capability crosses command startup and output dispatch, and it adds one dependency for `.env` parsing.

## Goals / Non-Goals

**Goals:**

- Make the configuration source visible in the output URI.
- Keep process environment values authoritative while filling missing values from `.env`.
- Limit `.env` loading and Elastic environment authentication to `env:/` output.
- Preserve the existing direct URL and configured-host paths.

**Non-Goals:**

- Add a flag for selecting a `.env` path.
- Add environment-backed input URIs.
- Change the names or formats of `ELASTIC_ES_URL` and `ELASTIC_ES_API_KEY`.
- Change known-host file discovery or authentication.

## Decisions

### Reserve only `env`

Output dispatch checks for the exact `env` scheme before direct URL and configured-host handling. `es` and `elasticsearch` take the configured-host path. This makes the special behavior explicit and avoids permanently consuming plausible cluster aliases.

Keeping the two existing schemes as deprecated aliases was considered. It would preserve compatibility, but it would keep the ambiguity and prevent those configured-host names from working.

### Load `.env` only for environment output

After parsing the output URI, command startup invokes `dotenvy` only when the scheme is `env`. The standard loader searches the working directory and its ancestors and does not replace variables already present in the process environment. Missing `.env` files are allowed. Parse and read errors other than absence stop startup.

Loading `.env` unconditionally at process startup was considered. It could change logging or authentication for direct URL and configured-host commands, which is outside this capability.

### Keep authentication precedence in command startup

Command-line authentication remains authoritative. The resolved environment API key is passed to authentication setup only for `env:/` output and only when no command-line authentication option is present. This keeps output construction independent of argument precedence rules.

### Validate and join the URL at the output boundary

The environment output branch parses `ELASTIC_ES_URL`, requires an absolute HTTP or HTTPS URL with a host, appends the target index to the configured base path, and removes query and fragment components. It then reuses the direct Elasticsearch output builder.

Building the target with string concatenation was considered, but URL parsing gives consistent validation and path handling before network work starts.

## Risks / Trade-offs

- [Existing commands using `es:/` or `elasticsearch:/` break] -> Document `env:/` as the migration and record the change as breaking.
- [A `.env` file in a parent directory supplies settings unexpectedly] -> Follow `dotenvy`'s documented nearest-file search and state that behavior in the capability spec and README.
- [Malformed `.env` content blocks an environment-backed command] -> Report the parsing error before input ingestion or network activity.
