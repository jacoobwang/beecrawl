from __future__ import annotations

import asyncio
import logging
import time
import uuid
from urllib.parse import urlparse

from beecrawl.models import (
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


class WebExtractionService:
    async def scrape(self, request: WebExtractScrapeRequest) -> WebExtractScrapeResponse:
        started = time.monotonic()
        request_id = _request_id()
        page = await self._fetch_page(request)
        markdown, metadata = http_static.extract_markdown(page.html, page.final_url)
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

    async def _fetch_page(self, request: WebExtractScrapeRequest):
        if request.use_browser == "always":
            return await asyncio.to_thread(
                browser.render_page, request.url, timeout_seconds=request.timeout_seconds
            )
        try:
            page = await asyncio.to_thread(
                http_static.fetch_page,
                request.url,
                timeout_seconds=request.timeout_seconds,
            )
            if request.use_browser == "auto" and _looks_like_render_needed(page.html):
                return await asyncio.to_thread(
                    browser.render_page, request.url, timeout_seconds=request.timeout_seconds
                )
            return page
        except WebExtractError as exc:
            if request.use_browser == "auto" and exc.retryable:
                return await asyncio.to_thread(
                    browser.render_page, request.url, timeout_seconds=request.timeout_seconds
                )
            raise


def _looks_like_render_needed(html: str) -> bool:
    text = (html or "").lower()
    return len(text.strip()) < 500 and any(marker in text for marker in ("<script", 'id="root"', 'id="app"'))


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
