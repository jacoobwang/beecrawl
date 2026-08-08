# Keep command results separate from diagnostics

BeeCrawl CLI will write only the command result to stdout and send progress, polling updates, warnings, and diagnostics to stderr. Commands will choose a useful default representation (`scrape` as Markdown and data-oriented commands as JSON), while `--format` and `--json` provide explicit machine-readable control for agents and scripts.
