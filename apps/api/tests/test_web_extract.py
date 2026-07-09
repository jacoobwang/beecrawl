from unittest.mock import patch

import pytest
from fastapi.testclient import TestClient

from beecrawl import app as app_module
from beecrawl.models import (
    WebExtractMapMetadata,
    WebExtractMapResponse,
    WebExtractMetadata,
    WebExtractScrapeResponse,
)
from beecrawl.web_extract.errors import blocked_by_policy, invalid_url
from beecrawl.web_extract.providers.http_static import extract_markdown, normalize_url


class FakeWebExtractService:
    async def scrape(self, request):
        return WebExtractScrapeResponse(
            request_id="webext_test",
            url=request.url,
            final_url="https://example.com/",
            markdown="# Example",
            metadata=WebExtractMetadata(
                title="Example",
                status_code=200,
                provider="http_static",
                rendered=False,
            ),
        )

    async def map_site(self, request):
        return WebExtractMapResponse(
            request_id="webext_test",
            url=request.url,
            links=["https://example.com/"],
            metadata=WebExtractMapMetadata(provider="html_links", count=1),
        )


def test_extract_markdown_keeps_title_headings_and_paragraphs() -> None:
    html = """
    <html lang="en"><head><title>Acme</title><script>bad()</script></head>
    <body><main><h1>About Acme</h1><p>We make durable parts.</p><li>ISO certified</li></main></body></html>
    """
    markdown, metadata = extract_markdown(html, "https://example.com")

    assert "# Acme" in markdown
    assert "# About Acme" in markdown
    assert "We make durable parts." in markdown
    assert "- ISO certified" in markdown
    assert metadata["title"] == "Acme"
    assert metadata["language"] == "en"


def test_extract_markdown_preserves_rich_markdown_structures() -> None:
    html = """
    <main>
      <p>This is <strong>bold</strong> and <em>italic</em>.</p>
      <table><tr><th>Product</th><th>Price</th></tr><tr><td>Widget</td><td>$5</td></tr></table>
      <pre><code>console.log("hi")</code></pre>
    </main>
    """
    markdown, _ = extract_markdown(html, "https://example.com")

    assert "This is **bold** and *italic*." in markdown
    assert "| Product | Price |" in markdown
    assert "| Widget | $5 |" in markdown
    assert 'console.log("hi")' in markdown


def test_extract_markdown_absolutizes_links_and_removes_layout_noise() -> None:
    html = """
    <html><body><main>
      <nav><a href="/skip">Navigation</a></nav>
      <p><a href="/about">About us</a></p>
      <p><a href="#content">Skip to Content</a></p>
      <footer>Copyright</footer>
    </main></body></html>
    """
    markdown, _ = extract_markdown(html, "https://example.com/products/page")

    assert "[About us](https://example.com/about)" in markdown
    assert "Navigation" not in markdown
    assert "Copyright" not in markdown
    assert "Skip to Content" not in markdown


def test_normalize_url_policy() -> None:
    assert normalize_url("example.com/path") == "https://example.com/path"

    with pytest.raises(type(blocked_by_policy()), match="Private network"):
        normalize_url("http://127.0.0.1:8000")

    with pytest.raises(type(invalid_url()), match="Only http"):
        normalize_url("ftp://example.com/file")


def test_web_extract_routes_return_contract_shape() -> None:
    client = TestClient(app_module.app)

    with (
        patch.object(app_module, "_web_extract_service", FakeWebExtractService()),
        patch.dict("os.environ", {"BEECRAWL_WEB_EXTRACT_API_KEY": "", "WEB_EXTRACT_API_KEY": ""}),
    ):
        scrape_response = client.post("/scrape", json={"url": "https://example.com"})
        map_response = client.post("/map", json={"url": "https://example.com"})
        removed_map_response = client.post("/web-extract/map", json={"url": "https://example.com"})
        removed_response = client.post("/web-extract/scrape", json={"url": "https://example.com"})
        removed_v1_response = client.post("/v1/scrape", json={"url": "https://example.com"})

    assert scrape_response.status_code == 200
    assert scrape_response.json()["request_id"] == "webext_test"
    assert scrape_response.json()["markdown"] == "# Example"
    assert scrape_response.json()["metadata"]["provider"] == "http_static"
    assert removed_response.status_code == 404
    assert removed_v1_response.status_code == 404

    assert map_response.status_code == 200
    assert map_response.json()["links"] == ["https://example.com/"]
    assert map_response.json()["metadata"]["count"] == 1
    assert removed_map_response.status_code == 404


def test_web_extract_route_requires_key_when_configured() -> None:
    client = TestClient(app_module.app)

    with (
        patch.object(app_module, "_web_extract_service", FakeWebExtractService()),
        patch.dict("os.environ", {"BEECRAWL_WEB_EXTRACT_API_KEY": "secret"}),
    ):
        denied = client.post("/scrape", json={"url": "https://example.com"})
        allowed = client.post(
            "/scrape",
            headers={"X-Web-Extract-Api-Key": "secret"},
            json={"url": "https://example.com"},
        )

    assert denied.status_code == 401
    assert allowed.status_code == 200
