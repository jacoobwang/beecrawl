from __future__ import annotations

from beecrawl.models import ProviderPage
from beecrawl.web_extract.errors import render_timeout
from beecrawl.web_extract.providers.http_static import extract_markdown, normalize_url


def render_page(url: str, *, timeout_seconds: int) -> ProviderPage:
    normalized = normalize_url(url)
    try:
        from playwright.sync_api import TimeoutError as PlaywrightTimeoutError
        from playwright.sync_api import sync_playwright
    except ImportError as exc:
        raise render_timeout("Playwright is not installed") from exc

    timeout_ms = int(timeout_seconds * 1000)
    try:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(headless=True)
            page = browser.new_page()
            response = page.goto(normalized, wait_until="networkidle", timeout=timeout_ms)
            html = page.content()
            final_url = page.url
            browser.close()
    except PlaywrightTimeoutError as exc:
        raise render_timeout() from exc
    except Exception as exc:
        raise render_timeout(str(exc)) from exc

    _, metadata = extract_markdown(html, final_url)
    return ProviderPage(
        url=normalized,
        final_url=final_url,
        html=html,
        status_code=response.status if response else None,
        title=metadata.get("title"),
        language=metadata.get("language"),
        provider="browser",
        rendered=True,
    )
