# Separate CLI output format from scrape content formats

`--format` will control how the CLI serializes its command result, while scrape request formats will use a separate repeatable `--content-format` flag. The distinction prevents a single option from ambiguously changing both the server request and the terminal representation.
