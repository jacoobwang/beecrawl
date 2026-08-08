# Keep the full command model while staging CLI delivery

BeeCrawl CLI will reserve a command model for the complete public API, but the first release will implement the agent-oriented core: `search`, `scrape`, `map`, `extract`, `crawl`, and `agent`. Document parsing, browser and scrape interaction, and monitors are deferred until the core output, authentication, and job handling are stable; this preserves future CLI compatibility without making the first release carry every stateful or file-upload workflow.
