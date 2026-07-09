import asyncio
from unittest.mock import patch

import pytest
from fastapi.testclient import TestClient

from beecrawl import app as app_module
from beecrawl.models import (
    ProviderPage,
    WebExtractMapMetadata,
    WebExtractMapResponse,
    WebExtractMetadata,
    WebExtractScrapeRequest,
    WebExtractScrapeResponse,
)
from beecrawl.web_extract.errors import blocked_by_policy, invalid_url
from beecrawl.web_extract.providers import browser, http_static
from beecrawl.web_extract.providers.http_static import discover_links, extract_markdown, normalize_url
from beecrawl.web_extract.service import WebExtractionService


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


def test_scrape_auto_falls_back_to_browser_for_short_static_content(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def fake_fetch_page(url: str, *, timeout_seconds: int) -> ProviderPage:
        return ProviderPage(
            url=url,
            final_url=url,
            html="<html><body><main><p>Loading...</p></main><script src='/app.js'></script></body></html>",
            status_code=200,
            provider="http_static",
        )

    def fake_render_page(url: str, *, timeout_seconds: int, wait_for_ms: int = 0) -> ProviderPage:
        return ProviderPage(
            url=url,
            final_url=url,
            html=f"<html><body><main><h1>Rendered</h1><p>{'Loaded content. ' * 80}</p></main></body></html>",
            status_code=200,
            provider="browser",
            rendered=True,
        )

    monkeypatch.setattr(http_static, "fetch_page", fake_fetch_page)
    monkeypatch.setattr(browser, "render_page", fake_render_page)

    response = asyncio.run(
        WebExtractionService().scrape(
            WebExtractScrapeRequest(url="https://example.com", use_browser="auto")
        )
    )

    assert response.metadata.provider == "browser"
    assert response.metadata.rendered is True
    assert "Loaded content." in response.markdown


def test_scrape_auto_keeps_sufficient_static_content(monkeypatch: pytest.MonkeyPatch) -> None:
    def fake_fetch_page(url: str, *, timeout_seconds: int) -> ProviderPage:
        return ProviderPage(
            url=url,
            final_url=url,
            html=f"<html><body><main><h1>Static</h1><p>{'Static content. ' * 80}</p></main></body></html>",
            status_code=200,
            provider="http_static",
        )

    def fail_render_page(url: str, *, timeout_seconds: int, wait_for_ms: int = 0) -> ProviderPage:
        raise AssertionError("browser fallback should not run")

    monkeypatch.setattr(http_static, "fetch_page", fake_fetch_page)
    monkeypatch.setattr(browser, "render_page", fail_render_page)

    response = asyncio.run(
        WebExtractionService().scrape(
            WebExtractScrapeRequest(url="https://example.com", use_browser="auto")
        )
    )

    assert response.metadata.provider == "http_static"
    assert response.metadata.rendered is False
    assert "Static content." in response.markdown


def test_scrape_always_passes_wait_for_ms_to_browser(monkeypatch: pytest.MonkeyPatch) -> None:
    captured: dict[str, int] = {}

    def fake_render_page(url: str, *, timeout_seconds: int, wait_for_ms: int = 0) -> ProviderPage:
        captured["wait_for_ms"] = wait_for_ms
        return ProviderPage(
            url=url,
            final_url=url,
            html="<html><body><main><p>Rendered page</p></main></body></html>",
            status_code=200,
            provider="browser",
            rendered=True,
        )

    monkeypatch.setattr(browser, "render_page", fake_render_page)

    response = asyncio.run(
        WebExtractionService().scrape(
            WebExtractScrapeRequest(
                url="https://example.com",
                use_browser="always",
                wait_for_ms=1234,
            )
        )
    )

    assert captured["wait_for_ms"] == 1234
    assert response.metadata.provider == "browser"


def test_map_include_merges_sitemap_and_html_links(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        http_static,
        "_discover_sitemap_links",
        lambda _url, *, limit: [
            "https://example.com/",
            "https://example.com/products?ref=sitemap",
            "https://cdn.example.com/asset",
        ],
    )
    monkeypatch.setattr(
        http_static,
        "_discover_html_links",
        lambda _url, *, limit: [
            "https://www.example.com/products?ref=nav#section",
            "https://example.com/about",
        ],
    )

    links, provider = discover_links(
        "https://example.com",
        search=None,
        limit=10,
        include_subdomains=False,
        sitemap="include",
        ignore_query_parameters=True,
    )

    assert provider == "sitemap+html_links"
    assert links == [
        "https://example.com/",
        "https://example.com/products",
        "https://example.com/about",
    ]


def test_map_sitemap_modes(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        http_static,
        "_discover_sitemap_links",
        lambda _url, *, limit: ["https://example.com/sitemap-page"],
    )
    monkeypatch.setattr(
        http_static,
        "_discover_html_links",
        lambda _url, *, limit: ["https://example.com/html-page"],
    )

    only_links, only_provider = discover_links(
        "https://example.com",
        search=None,
        limit=10,
        include_subdomains=False,
        sitemap="only",
        ignore_query_parameters=True,
    )
    skip_links, skip_provider = discover_links(
        "https://example.com",
        search=None,
        limit=10,
        include_subdomains=False,
        sitemap="skip",
        ignore_query_parameters=True,
    )

    assert only_links == ["https://example.com/sitemap-page"]
    assert only_provider == "sitemap"
    assert skip_links == ["https://example.com/html-page"]
    assert skip_provider == "html_links"
