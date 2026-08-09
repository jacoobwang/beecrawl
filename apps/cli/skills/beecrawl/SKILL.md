# BeeCrawl CLI

Use BeeCrawl CLI for web search, scraping, mapping, extraction, crawling, and
research Agents.

## Authentication

Run `beecrawl login` before the first API operation. The command opens the
BeeCrawl Dashboard, lets the user sign in and choose a workspace, and stores a
named local profile. Never ask the user to paste an API key into a Skill file,
URL, shell profile, or prompt. For automation, `BEECRAWL_API_KEY` is supported.

## Command selection

- `beecrawl search "query" --json` for web search and source discovery.
- `beecrawl scrape https://example.com` for one page; its default output is Markdown.
- `beecrawl map https://example.com --json` for links in a site.
- `beecrawl extract URL --schema-file schema.json --json` for structured data.
- `beecrawl crawl URL --json` for a completed site crawl.
- `beecrawl agent "research question" --json` for multi-source research.

Long-running commands wait by default. Use `crawl start`, `crawl status`,
`crawl cancel`, or the equivalent Agent lifecycle commands when the workflow
must be detached. `--no-wait` is a shortcut for the start behavior.

## Output and options

Use `--json` when consuming results programmatically. stdout contains only the
result; diagnostics and progress go to stderr. Use `--options-file` for nested
API options that do not have a dedicated CLI flag.
