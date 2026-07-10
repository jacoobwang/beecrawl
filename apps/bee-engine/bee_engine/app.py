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
