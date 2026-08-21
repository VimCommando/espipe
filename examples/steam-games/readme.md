# Steam Games Dataset

This example ingests the Kaggle Steam games dataset from March 2026 into a local Elasticsearch index named `steam-games`.

Dataset source:

https://www.kaggle.com/datasets/ebrucakar/steam-games-dataset-march-2026

Download and extract the archive, for example:

```text
~/Downloads/steam-games-dataset-march-2026/games.csv
```

## Ingest

From a repository checkout that includes this example, install the local `espipe` binary:

```sh
cargo install --path .
```

Then run from the repository root directory against a new `steam-games` index. If the index already exists, delete it first so Elasticsearch applies the template-defined default pipeline when the index is recreated:

```bash
espipe ~/Downloads/steam-games-dataset-march-2026/games.csv \
  http://localhost:9200/steam-games \
  --pipeline examples/steam-games/steam-games-pipeline.yml \
  --pipeline-name steam-games \
  --template examples/steam-games/steam-games-template.yml
```

The pipeline splits comma-delimited `Tags` and `Screenshots` values into arrays and converts `Windows`, `Mac`, and `Linux` from title-case strings into booleans.

### JSON object catalogue

The archive also contains `games.json`, whose root object uses Steam application IDs as property names. Stream its values into a separate index with:

```bash
espipe games.json \
  http://localhost:9200/steam-games-json \
  --split /
```

Each root property becomes one document, and its property name is added as the string `id` field. The JSON records already contain structured values, so this command does not use the CSV-specific pipeline or template above.

Split batches are transformed in parallel and may be emitted in any order. The generated string `id` preserves each root map key for downstream sorting or identity.

For a JSON export wrapped as `{"hits":[...]}`, select and drop the wrapper with:

```bash
espipe wrapped-games.json \
  http://localhost:9200/steam-games-json \
  --split /hits/
```

Array elements are emitted unchanged; unlike object properties, array positions do not generate `id` fields. Array output order is also unspecified.
