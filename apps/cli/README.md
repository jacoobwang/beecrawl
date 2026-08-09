# BeeCrawl CLI

The BeeCrawl CLI is the terminal client for the public BeeCrawl API.

```bash
npx -y beecrawl-cli@latest --help
beecrawl login
beecrawl search "web scraping" --limit 5
beecrawl scrape https://example.com
```

The CLI uses the v2 surface of `beecrawl-sdk`. `beecrawl login` opens the
Dashboard authorization flow and stores the resulting profile locally. API
keys can also be supplied with `BEECRAWL_API_KEY` or `--api-key`.
