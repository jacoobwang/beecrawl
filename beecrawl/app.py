from __future__ import annotations

import os

import httpx
from fastapi import FastAPI, HTTPException

from beecrawl.extractor import extract_fields, parse_html
from beecrawl.models import ExtractRequest, ExtractResponse, ScrapeRequest, ScrapeResponse

DEFAULT_USER_AGENT = "BeeCrawl/0.1 (+https://github.com/jacoobwang/beecrawl)"

app = FastAPI(
    title="BeeCrawl",
    description="Open-source infrastructure for crawling, extracting, and structuring web data.",
    version="0.1.0",
)


@app.post("/v1/scrape", response_model=ScrapeResponse)
async def scrape(request: ScrapeRequest) -> ScrapeResponse:
    html = await _fetch_html(str(request.url))
    return parse_html(str(request.url), html)


@app.post("/v1/extract", response_model=ExtractResponse)
async def extract(request: ExtractRequest) -> ExtractResponse:
    html = await _fetch_html(str(request.url))
    scrape_result = parse_html(str(request.url), html)
    return ExtractResponse(
        url=str(request.url),
        data=extract_fields(scrape_result, request.schema_),
        scrape=scrape_result,
    )


async def _fetch_html(url: str) -> str:
    headers = {"User-Agent": os.getenv("BEECRAWL_USER_AGENT", DEFAULT_USER_AGENT)}
    timeout = float(os.getenv("BEECRAWL_REQUEST_TIMEOUT_SECONDS", "20"))

    try:
        async with httpx.AsyncClient(follow_redirects=True, timeout=timeout, headers=headers) as client:
            response = await client.get(url)
            response.raise_for_status()
    except httpx.HTTPError as exc:
        raise HTTPException(status_code=502, detail=f"fetch_failed: {exc}") from exc

    content_type = response.headers.get("content-type", "")
    if "html" not in content_type.lower():
        raise HTTPException(status_code=415, detail=f"unsupported_content_type: {content_type}")

    return response.text
