# Markdown Conversion TODO

Current implementation walks the parsed HTML tree in document order and emits
local Markdown for headings, paragraphs, emphasis, links, images, lists,
blockquotes, code, and tables. Golden fixtures cover article and catalog-style
pages so DOM-order regressions fail tests.

Follow-up options:

- Add a `MarkdownConverter` provider interface so local conversion, HTTP
  conversion services, and future managed providers share one contract.
- Add an optional HTTP converter provider similar to Firecrawl's
  html-to-markdown service for heavier workloads.
- Expand golden fixtures with real-world docs sites, code-heavy pages, and
  malformed HTML.
- Add conversion metrics: input size, output size, elapsed time, empty-output
  rate, and fallback path.
- Revisit whether navigation/header/footer stripping should be configurable per
  domain once fixture coverage is broader.
