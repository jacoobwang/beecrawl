from __future__ import annotations

import asyncio
import contextlib
import os
from collections.abc import Awaitable, Callable
from typing import Any

from beecrawl.models import ProviderPage
from beecrawl.web_extract.errors import render_timeout
from beecrawl.web_extract.providers.http_static import extract_markdown, normalize_url

DEFAULT_MAX_PAGES = 4
BLOCKED_RESOURCE_TYPES = {"image", "media", "font"}
BLOCKED_HOST_PARTS = (
    "doubleclick.net",
    "adservice.google.com",
    "googlesyndication.com",
    "googletagmanager.com",
    "google-analytics.com",
)


class BrowserPool:
    def __init__(
        self,
        *,
        max_pages: int = DEFAULT_MAX_PAGES,
        playwright_factory: Callable[[], Any] | None = None,
    ) -> None:
        self._max_pages = max(1, max_pages)
        self._playwright_factory = playwright_factory
        self._playwright: Any | None = None
        self._browser: Any | None = None
        self._start_lock = asyncio.Lock()
        self._page_semaphore = asyncio.Semaphore(self._max_pages)

    async def render_page(self, url: str, *, timeout_seconds: int, wait_for_ms: int = 0) -> ProviderPage:
        normalized = normalize_url(url)
        timeout_ms = int(timeout_seconds * 1000)

        async with self._page_semaphore:
            browser = await self._get_browser()
            context = None
            page = None
            try:
                context = await browser.new_context(
                    viewport={"width": 1280, "height": 800},
                    service_workers="block",
                    user_agent=(
                        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
                        "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36"
                    ),
                )
                await context.route("**/*", _route_request)
                page = await context.new_page()
                response = await page.goto(normalized, wait_until="domcontentloaded", timeout=timeout_ms)
                with contextlib.suppress(Exception):
                    await page.wait_for_load_state("networkidle", timeout=min(5000, timeout_ms))
                if wait_for_ms > 0:
                    await page.wait_for_timeout(wait_for_ms)
                html = await page.content()
                final_url = page.url
            except Exception as exc:
                await self._recover_if_browser_closed()
                raise render_timeout(str(exc)) from exc
            finally:
                if page is not None:
                    with contextlib.suppress(Exception):
                        await page.close()
                if context is not None:
                    with contextlib.suppress(Exception):
                        await context.close()

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

    async def close(self) -> None:
        async with self._start_lock:
            if self._browser is not None:
                with contextlib.suppress(Exception):
                    await self._browser.close()
            if self._playwright is not None:
                with contextlib.suppress(Exception):
                    await self._playwright.stop()
            self._browser = None
            self._playwright = None

    async def _get_browser(self):
        if self._browser is not None and self._browser.is_connected():
            return self._browser
        async with self._start_lock:
            if self._browser is not None and self._browser.is_connected():
                return self._browser
            if self._playwright is None:
                self._playwright = await self._start_playwright()
            self._browser = await self._playwright.chromium.launch(
                headless=True,
                args=[
                    "--no-sandbox",
                    "--disable-setuid-sandbox",
                    "--disable-dev-shm-usage",
                    "--disable-accelerated-2d-canvas",
                    "--no-first-run",
                    "--no-zygote",
                    "--disable-gpu",
                ],
            )
            return self._browser

    async def _start_playwright(self):
        if self._playwright_factory is not None:
            return await self._maybe_await(self._playwright_factory())
        try:
            from playwright.async_api import async_playwright
        except ImportError as exc:
            raise render_timeout("Playwright is not installed") from exc
        return await async_playwright().start()

    async def _recover_if_browser_closed(self) -> None:
        if self._browser is not None and not self._browser.is_connected():
            await self.close()

    async def _maybe_await(self, value):
        if isinstance(value, Awaitable):
            return await value
        return value


async def render_page(url: str, *, timeout_seconds: int, wait_for_ms: int = 0) -> ProviderPage:
    return await _pool.render_page(url, timeout_seconds=timeout_seconds, wait_for_ms=wait_for_ms)


async def close_browser_pool() -> None:
    await _pool.close()


async def _route_request(route, request) -> None:
    try:
        hostname = request.url.split("/")[2].lower()
    except IndexError:
        hostname = ""
    if request.resource_type in BLOCKED_RESOURCE_TYPES or any(
        part in hostname for part in BLOCKED_HOST_PARTS
    ):
        await route.abort()
        return
    await route.continue_()


def _max_pages_from_env() -> int:
    try:
        return int(os.getenv("BEECRAWL_BROWSER_MAX_PAGES", str(DEFAULT_MAX_PAGES)))
    except ValueError:
        return DEFAULT_MAX_PAGES


_pool = BrowserPool(max_pages=_max_pages_from_env())
