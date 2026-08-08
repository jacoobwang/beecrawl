# Organize CLI commands around user tasks

BeeCrawl CLI will expose task-oriented verbs such as `search`, `scrape`, `map`, `extract`, `crawl`, and `agent`, with lifecycle subcommands only where a server-side Job needs explicit control. API versions, HTTP methods, and endpoint paths will remain implementation details so the CLI can evolve with the public API without forcing users or agents to learn transport-specific naming.
