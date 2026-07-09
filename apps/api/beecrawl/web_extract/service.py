from __future__ import annotations

import asyncio
import logging
import time
import uuid
from urllib.parse import urlparse

from beecrawl.models import (
    ProviderPage,
    WebExtractMapMetadata,
    WebExtractMapRequest,
    WebExtractMapResponse,
    WebExtractMetadata,
    WebExtractScrapeRequest,
    WebExtractScrapeResponse,
)
from beecrawl.web_extract.errors import WebExtractError, empty_content
from beecrawl.web_extract.providers import browser, http_static

logger = logging.getLogger(__name__)
MIN_AUTO_MARKDOWN_CHARS = 500


class WebExtractionService:
    async def scrape(self, request: WebExtractScrapeRequest) -> WebExtractScrapeResponse:
        started = time.monotonic()
        request_id = _request_id()
        page, markdown, metadata = await self._scrape_page(request, request_id)
        if not markdown:
            raise empty_content()
        elapsed_ms = _elapsed_ms(started)
        logger.info(
            "web_extract.scrape.completed",
            extra=_log_extra(request_id, page.final_url, page.provider, elapsed_ms),
        )
        return WebExtractScrapeResponse(
            request_id=request_id,
            url=page.url,
            final_url=page.final_url,
            markdown=markdown,
            metadata=WebExtractMetadata(
                title=metadata.get("title") or page.title,
                language=metadata.get("language") or page.language,
                status_code=page.status_code,
                provider=page.provider,
                rendered=page.rendered,
                elapsed_ms=elapsed_ms,
            ),
        )

    async def map_site(self, request: WebExtractMapRequest) -> WebExtractMapResponse:
        started = time.monotonic()
        request_id = _request_id()
        links, provider = await asyncio.to_thread(
            http_static.discover_links,
            request.url,
            search=request.search,
            limit=request.limit,
            include_subdomains=request.include_subdomains,
            sitemap="skip" if request.ignore_sitemap else request.sitemap,
            ignore_query_parameters=request.ignore_query_parameters,
        )
        elapsed_ms = _elapsed_ms(started)
        logger.info(
            "web_extract.map.completed",
            extra=_log_extra(request_id, request.url, provider, elapsed_ms, count=len(links)),
        )
        return WebExtractMapResponse(
            request_id=request_id,
            url=http_static.normalize_url(request.url),
            links=links,
            metadata=WebExtractMapMetadata(provider=provider, count=len(links), elapsed_ms=elapsed_ms),
        )

    async def _scrape_page(
        self,
        request: WebExtractScrapeRequest,
        request_id: str,
    ) -> tuple[ProviderPage, str, dict[str, str | None]]:
        if request.use_browser == "always":
            page = await self._render_page(request)
            return _page_to_markdown(page)

        try:
            page = await asyncio.to_thread(
                http_static.fetch_page,
                request.url,
                timeout_seconds=request.timeout_seconds,
            )
        except WebExtractError as exc:
            if request.use_browser == "auto" and exc.retryable:
                page = await self._render_page(request)
                return _page_to_markdown(page)
            raise

        _, markdown, metadata = _page_to_markdown(page)
        if request.use_browser != "auto" or _is_markdown_sufficient(markdown):
            return page, markdown, metadata

        try:
            browser_page = await self._render_page(request)
        except WebExtractError as exc:
            logger.info(
                "web_extract.scrape.browser_fallback_failed",
                extra={
                    **_log_extra(request_id, page.final_url, page.provider, 0),
                    "error": exc.message,
                },
            )
            raise

        _, browser_markdown, browser_metadata = _page_to_markdown(browser_page)
        if _is_better_markdown(browser_markdown, markdown):
            return browser_page, browser_markdown, browser_metadata
        return page, markdown, metadata

    async def _render_page(self, request: WebExtractScrapeRequest) -> ProviderPage:
        return await asyncio.to_thread(
            browser.render_page,
            request.url,
            timeout_seconds=request.timeout_seconds,
            wait_for_ms=request.wait_for_ms,
        )


def _page_to_markdown(page: ProviderPage) -> tuple[ProviderPage, str, dict[str, str | None]]:
    markdown, metadata = http_static.extract_markdown(page.html, page.final_url)
    return page, markdown, metadata


def _is_markdown_sufficient(markdown: str) -> bool:
    return len(markdown.strip()) >= MIN_AUTO_MARKDOWN_CHARS


def _is_better_markdown(candidate: str, current: str) -> bool:
    candidate_length = len(candidate.strip())
    current_length = len(current.strip())
    return candidate_length > 0 and candidate_length >= max(MIN_AUTO_MARKDOWN_CHARS, current_length * 2)


def _request_id() -> str:
    return f"webext_{uuid.uuid4().hex[:16]}"


def _elapsed_ms(started: float) -> int:
    return int((time.monotonic() - started) * 1000)


def _log_extra(request_id: str, url: str, provider: str, elapsed_ms: int, **extra: object) -> dict[str, object]:
    return {
        "request_id": request_id,
        "provider": provider,
        "elapsed_ms": elapsed_ms,
        "url_host": urlparse(url).hostname or "",
        **extra,
    }
