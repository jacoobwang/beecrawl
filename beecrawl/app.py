from __future__ import annotations

import os

import httpx
from fastapi import Depends, FastAPI, Header, HTTPException

from beecrawl.extractor import extract_fields, parse_html
from beecrawl.models import (
    ExtractRequest,
    ExtractResponse,
    WebExtractMapRequest,
    WebExtractMapResponse,
    WebExtractScrapeRequest,
    WebExtractScrapeResponse,
)
from beecrawl.web_extract.errors import WebExtractError
from beecrawl.web_extract.service import WebExtractionService

DEFAULT_USER_AGENT = "BeeCrawl/0.1 (+https://github.com/jacoobwang/beecrawl)"

app = FastAPI(
    title="BeeCrawl",
    description="Open-source infrastructure for crawling, extracting, and structuring web data.",
    version="0.1.0",
)
_web_extract_service = WebExtractionService()


def _require_web_extract_auth(
    authorization: str | None = Header(default=None),
    x_api_key: str | None = Header(default=None),
    x_web_extract_api_key: str | None = Header(default=None),
) -> None:
    api_key = (
        os.getenv("BEECRAWL_WEB_EXTRACT_API_KEY", "").strip()
        or os.getenv("WEB_EXTRACT_API_KEY", "").strip()
    )
    if not api_key:
        return
    bearer = ""
    if authorization and authorization.lower().startswith("bearer "):
        bearer = authorization.split(" ", 1)[1].strip()
    supplied = x_web_extract_api_key or x_api_key or bearer
    if supplied != api_key:
        raise HTTPException(
            status_code=401,
            detail={"code": "unauthorized", "message": "Invalid web extraction API key", "retryable": False},
        )


@app.post(
    "/v1/scrape",
    response_model=WebExtractScrapeResponse,
    summary="Extract Markdown from a URL",
    response_description="Markdown extraction result",
)
async def scrape(
    request: WebExtractScrapeRequest,
    _: None = Depends(_require_web_extract_auth),
) -> WebExtractScrapeResponse:
    try:
        return await _web_extract_service.scrape(request)
    except WebExtractError as exc:
        raise HTTPException(status_code=exc.http_status, detail=exc.to_detail()) from exc


@app.post("/extract", response_model=ExtractResponse)
async def extract(request: ExtractRequest) -> ExtractResponse:
    html = await _fetch_html(str(request.url))
    scrape_result = parse_html(str(request.url), html)
    return ExtractResponse(
        url=str(request.url),
        data=extract_fields(scrape_result, request.schema_),
        scrape=scrape_result,
    )


@app.post(
    "/map",
    response_model=WebExtractMapResponse,
    summary="Discover URLs for a site",
    response_description="Discovered site URLs",
)
async def map_web_site(
    request: WebExtractMapRequest,
    _: None = Depends(_require_web_extract_auth),
) -> WebExtractMapResponse:
    try:
        return await _web_extract_service.map_site(request)
    except WebExtractError as exc:
        raise HTTPException(status_code=exc.http_status, detail=exc.to_detail()) from exc


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
