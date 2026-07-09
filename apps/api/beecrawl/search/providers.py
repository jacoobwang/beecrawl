from __future__ import annotations

from dataclasses import dataclass
import os
from urllib.parse import parse_qs, unquote, urlparse

from bs4 import BeautifulSoup
import httpx


DEFAULT_USER_AGENT = "BeeCrawl/0.1 (+https://github.com/jacoobwang/beecrawl)"


@dataclass(frozen=True)
class SearchProviderResult:
    url: str
    title: str | None = None
    description: str | None = None


@dataclass(frozen=True)
class SearchProviderResponse:
    provider: str
    results: list[SearchProviderResult]


async def search_web(
    query: str,
    *,
    limit: int,
    lang: str = "en",
    country: str = "us",
) -> SearchProviderResponse:
    searxng_endpoint = os.getenv("BEECRAWL_SEARXNG_ENDPOINT", "").strip()
    if searxng_endpoint:
        response = await _search_searxng(
            searxng_endpoint,
            query,
            limit=limit,
            lang=lang,
        )
        if response.results:
            return response

    return await _search_duckduckgo(query, limit=limit, lang=lang, country=country)


async def _search_searxng(
    endpoint: str,
    query: str,
    *,
    limit: int,
    lang: str,
) -> SearchProviderResponse:
    url = endpoint.rstrip("/") + "/search"
    timeout = float(os.getenv("BEECRAWL_SEARCH_TIMEOUT_SECONDS", "10"))
    params = {
        "q": query,
        "language": lang,
        "format": "json",
    }
    engines = os.getenv("BEECRAWL_SEARXNG_ENGINES", "").strip()
    categories = os.getenv("BEECRAWL_SEARXNG_CATEGORIES", "").strip()
    if engines:
        params["engines"] = engines
    if categories:
        params["categories"] = categories

    try:
        async with httpx.AsyncClient(timeout=timeout, follow_redirects=True) as client:
            response = await client.get(url, params=params)
            response.raise_for_status()
    except httpx.HTTPError:
        return SearchProviderResponse(provider="searxng", results=[])

    payload = response.json()
    results = []
    for item in payload.get("results", [])[:limit]:
        item_url = item.get("url")
        if not item_url:
            continue
        results.append(
            SearchProviderResult(
                url=item_url,
                title=item.get("title"),
                description=item.get("content"),
            )
        )

    return SearchProviderResponse(provider="searxng", results=results)


async def _search_duckduckgo(
    query: str,
    *,
    limit: int,
    lang: str,
    country: str,
) -> SearchProviderResponse:
    timeout = float(os.getenv("BEECRAWL_SEARCH_TIMEOUT_SECONDS", "10"))
    params = {
        "q": query,
        "kp": "1",
        "kl": f"{country.lower()}-{lang.lower()}",
    }
    headers = {
        "User-Agent": os.getenv("BEECRAWL_USER_AGENT", DEFAULT_USER_AGENT),
        "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        "Accept-Language": "en-US,en;q=0.5",
    }
    try:
        async with httpx.AsyncClient(timeout=timeout, follow_redirects=True, headers=headers) as client:
            response = await client.get("https://html.duckduckgo.com/html", params=params)
            response.raise_for_status()
    except httpx.HTTPError:
        return SearchProviderResponse(provider="duckduckgo", results=[])

    soup = BeautifulSoup(response.text, "html.parser")
    results: list[SearchProviderResult] = []
    seen: set[str] = set()
    for result in soup.select(".result"):
        link = result.select_one(".result__a")
        if not link:
            continue
        href = link.get("href")
        url = _decode_duckduckgo_url(href or "")
        if not url or url in seen:
            continue
        seen.add(url)
        snippet = result.select_one(".result__snippet")
        results.append(
            SearchProviderResult(
                url=url,
                title=link.get_text(" ", strip=True) or None,
                description=snippet.get_text(" ", strip=True) if snippet else None,
            )
        )
        if len(results) >= limit:
            break

    return SearchProviderResponse(provider="duckduckgo", results=results)


def _decode_duckduckgo_url(href: str) -> str:
    if not href:
        return ""
    parsed = urlparse(href)
    if parsed.netloc.endswith("duckduckgo.com") or parsed.path.startswith("/l/"):
        uddg = parse_qs(parsed.query).get("uddg")
        if uddg:
            return unquote(uddg[0])
    return href
