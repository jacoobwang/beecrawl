# Make long-running CLI commands blocking by default

`crawl` and `agent` commands will submit their server-side Job and wait for a terminal result by default, because a single shell invocation should produce the requested data for both humans and coding agents. Explicit `start`, `status`, and `cancel` subcommands, plus `--no-wait`, will expose detached Job control for CI, long-running work, and callers that need to manage lifecycle separately.
