# Treat a bare HTTP URL as a scrape command

When the first non-option CLI argument is a valid `http` or `https` URL and is not an explicit management command, BeeCrawl CLI will interpret it as `scrape`. The shorthand is limited to URLs; other input is never guessed as search or agent work, preserving predictable command dispatch.
