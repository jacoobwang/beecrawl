from __future__ import annotations

import contextlib

from beecrawl.models import ProviderPage
from beecrawl.web_extract.errors import render_timeout
from beecrawl.web_extract.providers.http_static import extract_markdown, normalize_url


def render_page(url: str, *, timeout_seconds: int, wait_for_ms: int = 0) -> ProviderPage:
    normalized = normalize_url(url)
    try:
        from playwright.sync_api import TimeoutError as PlaywrightTimeoutError
        from playwright.sync_api import sync_playwright
    except ImportError as exc:
        raise render_timeout("Playwright is not installed") from exc

    timeout_ms = int(timeout_seconds * 1000)
    browser_instance = None
    try:
        with sync_playwright() as playwright:
            browser_instance = playwright.chromium.launch(headless=True)
            page = browser_instance.new_page()
            response = page.goto(normalized, wait_until="domcontentloaded", timeout=timeout_ms)
            with contextlib.suppress(PlaywrightTimeoutError):
                page.wait_for_load_state("networkidle", timeout=min(5000, timeout_ms))
            if wait_for_ms > 0:
                page.wait_for_timeout(wait_for_ms)
            html = page.content()
            final_url = page.url
            browser_instance.close()
    except PlaywrightTimeoutError as exc:
        raise render_timeout() from exc
    except Exception as exc:
        raise render_timeout(str(exc)) from exc
    finally:
        if browser_instance is not None:
            with contextlib.suppress(Exception):
                browser_instance.close()

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
