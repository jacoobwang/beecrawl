from __future__ import annotations

from collections.abc import AsyncIterator
from contextlib import asynccontextmanager

from fastapi import FastAPI, HTTPException

from bee_engine.browser import BrowserPool
from bee_engine.jobs import JobStore
from bee_engine.models import (
    BeeEngineScrapeRequest,
    BeeEngineScrapeResponse,
    BeeEngineStatusResponse,
    FingerprintFetchRequest,
    FingerprintFetchResponse,
    ProcessingResponse,
)

_browser_pool = BrowserPool()
_job_store = JobStore(_browser_pool)


@asynccontextmanager
async def lifespan(_app: FastAPI) -> AsyncIterator[None]:
    try:
        yield
    finally:
        await _browser_pool.close()


app = FastAPI(
    title="Bee Engine",
    description="Browser rendering service for Beecrawl.",
    version="0.1.0",
    lifespan=lifespan,
)


@app.get("/health")
async def health() -> dict[str, bool]:
    return {"ok": True}


@app.post("/fetch", response_model=FingerprintFetchResponse)
async def fingerprint_fetch(request: FingerprintFetchRequest) -> FingerprintFetchResponse:
    try:
        from curl_cffi.requests import AsyncSession
    except ImportError as exc:
        raise HTTPException(
            status_code=503,
            detail="Install the fingerprint extra to enable TLS impersonation",
        ) from exc

    proxy = _proxy_url(request.proxy) if request.proxy else None
    try:
        async with AsyncSession(impersonate=request.profile.replace("_", "")) as session:
            response = await session.request(
                request.method,
                request.url,
                headers=request.headers,
                proxy=proxy,
                timeout=request.timeout_ms / 1000,
                verify=not request.skip_tls_verification,
                allow_redirects=True,
            )
    except Exception as exc:
        raise HTTPException(status_code=502, detail=f"Fingerprint fetch failed: {exc}") from exc
    return FingerprintFetchResponse(
        status=response.status_code,
        url=str(response.url),
        headers={str(key): str(value) for key, value in response.headers.items()},
        body=response.text,
    )


def _proxy_url(proxy) -> str:
    if not proxy.username:
        return proxy.server
    from urllib.parse import quote, urlsplit, urlunsplit

    parsed = urlsplit(proxy.server)
    credentials = quote(proxy.username, safe="")
    if proxy.password is not None:
        credentials += f":{quote(proxy.password, safe='')}"
    host = parsed.hostname or ""
    if parsed.port:
        host += f":{parsed.port}"
    return urlunsplit((parsed.scheme, f"{credentials}@{host}", parsed.path, parsed.query, ""))


@app.post("/scrape", response_model=BeeEngineScrapeResponse | ProcessingResponse)
async def scrape(request: BeeEngineScrapeRequest) -> BeeEngineScrapeResponse | ProcessingResponse:
    if request.engine not in {"playwright", "chrome-cdp"}:
        raise HTTPException(status_code=400, detail="Unsupported engine")
    if request.instant_return:
        return await _job_store.enqueue(request)
    return await _job_store.run_sync(request)


@app.get("/scrape/{job_id}", response_model=BeeEngineStatusResponse)
async def scrape_status(job_id: str) -> BeeEngineStatusResponse:
    status = await _job_store.get(job_id)
    if status is None:
        raise HTTPException(status_code=404, detail="Scrape job not found")
    return status


@app.delete("/scrape/{job_id}")
async def delete_scrape(job_id: str) -> dict[str, bool]:
    deleted = await _job_store.delete(job_id)
    if not deleted:
        raise HTTPException(status_code=404, detail="Scrape job not found")
    return {"success": True}
