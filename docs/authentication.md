---
type: Reference
title: Authentication
description: Elastic CLI contexts, environment targets, known hosts, and credentials for direct Elasticsearch URLs.
resource: https://github.com/VimCommando/espipe/blob/main/docs/authentication.md
tags:
  - espipe
  - authentication
  - elasticsearch
  - cli
status: stable
---

# Authentication

These settings apply to the Elasticsearch destinations described in
[Output](output.md).

## Elastic CLI context targets

Use a leading-dot output target to read an Elasticsearch URL and credentials from `.elasticrc` without changing the file:

```bash
espipe docs.ndjson .es:/my-index
espipe docs.ndjson .production.es:/my-index
espipe docs.ndjson .production.us-west.elasticsearch:/my-index
```

The rightmost segment selects the application. Context outputs accept `es` and `elasticsearch`; espipe rejects Kibana, Cloud, and unknown applications because bulk requests require Elasticsearch.

`ELASTIC_CLI_CONFIG_FILE` takes precedence when set. Otherwise, espipe searches the user's home directory for the first readable `.elasticrc`, `.elasticrc.json`, `.elasticrc.yaml`, or `.elasticrc.yml`. An active target such as `.es:/my-index` uses `current_context`. A named target selects the context before the application segment.

The context may use inline authentication or any resolver supported by `elasticrc`, including environment, file, OS credential store, `pass`, and command resolvers. Resolver-backed values come from trusted local configuration; command resolvers run programs without a shell and with time and output limits. espipe resolves only the selected Elasticsearch service and does not write the config file.

Context API-key or basic authentication is used when the command has no authentication flags. `--apikey` or a complete `--username` and `--password` pair takes precedence. `--insecure` still controls certificate validation.

## Environment targets

The `env:/index` output reads its connection settings from:

- `ELASTIC_ES_URL` supplies the Elasticsearch base URL.
- `ELASTIC_ES_API_KEY` supplies API-key authentication when no `--apikey`, `--username`, or `--password` option is provided.

Values already present in the process environment take precedence. For missing values, `espipe` searches the current directory and its parents for a `.env` file. The command fails if `ELASTIC_ES_URL` remains unset. This also works with environment variables supplied by an [Elastic CLI extension](https://github.com/elastic/cli).

```bash
espipe docs.ndjson env:/my-index
```

## Known hosts file

Known hosts load from `$ESPIPE_HOSTS` or, by default, `~/.espipe/hosts.yml`.

Example:

```yaml
localhost:
  auth: None
  url: http://localhost:9200/

secure-cluster:
  auth: Basic
  url: https://example.com:9200/
  username: elastic
  password: changeme
  insecure: false

ess-cluster:
  auth: ApiKey
  url: https://cluster.example.com/
  apikey: "base64-encoded-api-key"
```

Usage:

```bash
espipe docs.ndjson localhost:my-index
espipe docs.ndjson secure-cluster:my-index
espipe docs.ndjson ess-cluster:my-index
```

Known-host outputs use the authentication and TLS settings from their host entry.

## Authentication arguments

These flags only apply to direct HTTP and HTTPS Elasticsearch URL outputs:

- `--apikey`
- `--username`
- `--password`
- `--insecure`
