# Expose common scrape options as first-class flags

The first CLI release will expose content formats, request timeout, browser wait time, main-content filtering, clean-content filtering, and include or exclude tags as discoverable scrape flags. Less common request fields remain available through the JSON options escape hatch so the CLI stays usable without duplicating the full API schema in its top-level grammar.
