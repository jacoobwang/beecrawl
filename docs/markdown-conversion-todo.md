# Markdown Conversion TODO

Current implementation uses local `markdownify` conversion with BeeCrawl cleanup
and post-processing.

Follow-up options:

- Add a `MarkdownConverter` provider interface so local conversion, HTTP
  conversion services, and future managed providers share one contract.
- Add an optional HTTP converter provider similar to Firecrawl's
  html-to-markdown service for heavier workloads.
- Expand golden fixtures with real-world pages that cover docs sites, ecommerce
  pages, tables, code-heavy pages, and malformed HTML.
- Add conversion metrics: input size, output size, elapsed time, empty-output
  rate, and fallback path.
- Revisit whether navigation/header/footer stripping should be configurable per
  domain once fixture coverage is broader.
