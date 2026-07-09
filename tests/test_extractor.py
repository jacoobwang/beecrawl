from beecrawl.extractor import extract_fields, parse_html


HTML = """
<html>
  <head>
    <title>Example Inc</title>
    <meta name="description" content="A demo company">
  </head>
  <body>
    <h1>Example Inc</h1>
    <p>Email sales@example.com or call +1 555 123 4567.</p>
    <a href="/about">About</a>
  </body>
</html>
"""


def test_parse_html_extracts_text_links_and_metadata() -> None:
    result = parse_html("https://example.com", HTML)

    assert result.title == "Example Inc"
    assert "sales@example.com" in result.text
    assert result.links[0].url == "https://example.com/about"
    assert result.metadata["description"] == "A demo company"


def test_extract_fields_uses_deterministic_baseline_rules() -> None:
    scrape = parse_html("https://example.com", HTML)
    result = extract_fields(scrape, {"company_name": "Company", "email": "Email", "phone": "Phone"})

    assert result == {
        "company_name": "Example Inc",
        "email": "sales@example.com",
        "phone": "+1 555 123 4567",
    }
