# Steam Games Dataset

This example ingests the Kaggle Steam games dataset from March 2026 into a local Elasticsearch index named `steam-games`.

Dataset source:

https://www.kaggle.com/datasets/ebrucakar/steam-games-dataset-march-2026

Download and extract the archive, for example:

```text
~/Downloads/steam-games-dataset-march-2026/games.csv
```

## Ingest

From a repository checkout, install the `espipe` binary:

```sh
# from local source
cargo install --path .

# Or install a released binary, then run the command below from this checkout
cargo install espipe
brew install VimCommando/tools/espipe
```

Then from the repository root directory run:

```bash
espipe ~/Downloads/steam-games-dataset-march-2026/games.csv \
  http://localhost:9200/steam-games \
  --pipeline examples/steam-games/steam-games-pipeline.yml \
  --pipeline-name steam-games \
  --template examples/steam-games/steam-games-template.yml
```

The pipeline splits comma-delimited `Tags` and `Screenshots` values into arrays and converts `Windows`, `Mac`, and `Linux` from title-case strings into booleans.
