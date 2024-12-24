# Elasticsearch document pipe (espipe)

A minimalist command-line utility to pipe documents from a file or I/O stream into an Elasticsearch cluster.

Have you ever had thousands of sample documents in a file, and you just want to load them all into an unsecure local Elasticsearch cluster?

```bash
espipe docs.ndjson http://localhost:9200/new_index
```

And you're done.

## Description

The goal of `espipe` is to provide the simpliest way to bulk-load a dataset into Elasticsearch. It does not do any document trasnformation or enrichment, and only requires the inputs be valid, deserializable JSON objects in a newline-dilemited json (`.ndjson`) file or comma-separated value (`.csv`) file.

It is multi-threaded and capable of fully saturating the CPU of the sending host. This could potentially overwhelm the target cluster, so use with caution on large data sets.

Documents are batched into `_bulk` requests of 5,000 documents and sent with the `create` action. It is not opinionated if the target is an alias, regular index or a data stream; just define your index templates and ingest pipelines in advance.

## Installation

1. Make sure you have `cargo` installed from [rust-lang.org](https://doc.rust-lang.org/cargo/getting-started/installation.html)
2. Clone this repository to your local machine
3. From the repository directory, run `cargo install --path .`

## Usage

```bash
Usage: espipe [OPTIONS] <INPUT> <OUTPUT>

Arguments:
  <INPUT>   The input URI to read docs from
  <OUTPUT>  The output URI to send docs to

Options:
  -k, --insecure             Ignore certificate validation
  -a, --apikey <APIKEY>      Apikey to authenticate via http header
  -u, --username <USERNAME>  Username for authentication
  -p, --password <PASSWORD>  Password for authentication
  -q, --quiet                Quiet mode, don't print runtime summary
  -h, --help                 Print help
```

### Arguments

Both the `<INPUT>` and `<OUTPUT>` arguments are URI-formatted strings.

The input URI can be a:
1. A stream from `stdin`: `-`
2. An unqualified file path: `file.ext`, `~/dir/file.ext`
3. A fully-qualified `file://` scheme URI: `file:///Users/name/dir/file.ext`

The output URI can be:
1. A stream to `stdout`: `-`
2. An unqualified file path: `file.ext`, `~/dir/file.ext`
3. A fully-qualified `file://` scheme URI: `file:///Users/name/dir/file.ext`
4. An `http://` or `https://` scheme URL to an Elasticsearch cluster, including index name: `http://example.com/index_name`
5. A known host saved in the `~/.esdiag/hosts.yml` configuration file: `localhost:index_name`

When piping to an Elasticsearch output, the index name is required.

### Options

All authentication options only apply to an http(s) output.

## Known Hosts configuration

You may create an `~/.esdiag/hosts.yml` configuration file to much like an `~/.ssh/config` file.

For example, here is a `localhost` definition with no authentication:

```yaml
localhost:
  auth: None
  url: http://localhost:9200/
```

This allows you to use `localhost` as a shorthand for `http://localhost:9200/`. Both commands are equivalent:

```bash
espipe docs.ndjson http://localhost:9200/new_index
espipe docs.ndjson localhost:new_index
```

An Elasticsearch Service (ESS) cluster with API key authentication:

```yaml
ess-cluster:
  auth: Apikey
  url: https://ess-cluster.es.us-west-2.aws.found.io/
  apikey: "fak34p1k3ydcbcc2c134c3eb3bf967bcf67q=="
```

Enabling you to use the shorthand:

```bash
espipe docs.ndjson https://esdiag.es.us-west-2.aws.found.io/new_index --apikey="fak34p1k3ydcbcc2c134c3eb3bf967bcf67q=="
espipe docs.ndjson ess-cluster:new_index
```

## Troubleshooting

If you need detailed logs on what `espipe` is doing, you can set the `RUST_LOG` environment variable:

```bash
export RUST_LOG=debug
```

## Examples

### Load a single `.ndjson` file into an Elastic Cloud cluster using an API key:

```bash
espipe docs.ndjson https://esdiag.es.us-west-2.aws.found.io/new_index --apikey="fak34p1k3ydcbcc2c134c3eb3bf967bcf67q=="
```

### Load all `.ndjson` files from an Agent diagnostics into a local Elasticsearch cluster:

1. Define a shell function that finds all `.ndjson` files recursively, calling `espipe` on each:

    ```bash
    function espipe-find() { for file in $(find $1 -name "*.ndjson" ); do echo -n "$file > "; espipe "$file" "$2"; done }
    ```

2. The `espipe-find` function with the directory and output target index matching the `logs-*-*` datastream template:

    ```bash
    espipe-find elastic-agent-123abc http://localhost:9200/logs-agent-default
    ```

This ingests all documents into a new datastream called `logs-agent-default` making the logs visible in Kibana's logs explorer.
