from __future__ import annotations

import asyncio
import time
import uuid

from beecrawl.models import (
    SearchMetadata,
    SearchRequest,
    SearchResponse,
    SearchResult,
    WebExtractScrapeRequest,
)
from beecrawl.search import providers
from beecrawl.web_extract.errors import WebExtractError
from beecrawl.web_extract.service import WebExtractionService


class SearchService:
    def __init__(self, web_extract_service: WebExtractionService | None = None) -> None:
        self._web_extract_service = web_extract_service or WebExtractionService()

    async def search(self, request: SearchRequest) -> SearchResponse:
        started = time.monotonic()
        request_id = f"search_{uuid.uuid4().hex[:16]}"
        provider_response = await providers.search_web(
            request.query,
            limit=request.limit,
            lang=request.lang,
            country=request.country,
        )
        results = [
            SearchResult(url=item.url, title=item.title, description=item.description)
            for item in provider_response.results[: request.limit]
        ]

        if _should_scrape(request):
            await asyncio.gather(*(self._scrape_result(result, request) for result in results))

        scraped_count = sum(1 for result in results if result.markdown)
        return SearchResponse(
            requestId=request_id,
            query=request.query,
            results=results,
            metadata=SearchMetadata(
                provider=provider_response.provider,
                count=len(results),
                scrapedCount=scraped_count,
                elapsedMs=_elapsed_ms(started),
            ),
        )

    async def _scrape_result(self, result: SearchResult, request: SearchRequest) -> None:
        assert request.scrape_options is not None
        options = request.scrape_options
        try:
            scrape = await self._web_extract_service.scrape(
                WebExtractScrapeRequest(
                    url=result.url,
                    formats=options.formats,
                    timeout_seconds=options.timeout_seconds,
                    wait_for_ms=options.wait_for_ms,
                    use_browser=options.use_browser,
                )
            )
        except WebExtractError as exc:
            result.scrape_error = exc.code
            return

        result.markdown = scrape.markdown
        result.metadata = {
            "final_url": scrape.final_url,
            "title": scrape.metadata.title,
            "provider": scrape.metadata.provider,
            "rendered": scrape.metadata.rendered,
            "status_code": scrape.metadata.status_code,
        }


def _should_scrape(request: SearchRequest) -> bool:
    return bool(request.scrape_options and request.scrape_options.formats)


def _elapsed_ms(started: float) -> int:
    return int((time.monotonic() - started) * 1000)
