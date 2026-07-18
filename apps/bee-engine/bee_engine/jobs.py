from __future__ import annotations

import asyncio
import time
import uuid

from bee_engine.browser import BrowserPool
from bee_engine.models import (
    BeeEngineScrapeRequest,
    BeeEngineScrapeResponse,
    BeeEngineStatusResponse,
    FailedResponse,
    ProcessingResponse,
)


class JobStore:
    def __init__(self, browser_pool: BrowserPool) -> None:
        self._browser_pool = browser_pool
        self._jobs: dict[str, BeeEngineStatusResponse] = {}
        self._lock = asyncio.Lock()

    async def run_sync(self, request: BeeEngineScrapeRequest) -> BeeEngineScrapeResponse:
        started = time.monotonic()
        response = await asyncio.wait_for(
            self._browser_pool.render(request), timeout=request.timeout / 1000
        )
        response.time_taken = _elapsed_ms(started)
        return response

    async def enqueue(self, request: BeeEngineScrapeRequest) -> ProcessingResponse:
        job_id = request.scrape_id or uuid.uuid4().hex
        async with self._lock:
            self._jobs[job_id] = ProcessingResponse(jobId=job_id)
        asyncio.create_task(self._run_job(job_id, request))
        return ProcessingResponse(jobId=job_id)

    async def get(self, job_id: str) -> BeeEngineStatusResponse | None:
        async with self._lock:
            return self._jobs.get(job_id)

    async def delete(self, job_id: str) -> bool:
        async with self._lock:
            return self._jobs.pop(job_id, None) is not None

    async def health(self) -> dict[str, int]:
        async with self._lock:
            processing = sum(
                1 for job in self._jobs.values() if getattr(job, "processing", False)
            )
            failed = sum(1 for job in self._jobs.values() if isinstance(job, FailedResponse))
            return {
                "total": len(self._jobs),
                "processing": processing,
                "completed": len(self._jobs) - processing - failed,
                "failed": failed,
            }

    async def _run_job(self, job_id: str, request: BeeEngineScrapeRequest) -> None:
        try:
            result = await self.run_sync(request)
            result.job_id = job_id
            async with self._lock:
                self._jobs[job_id] = result
        except Exception as exc:
            async with self._lock:
                self._jobs[job_id] = FailedResponse(jobId=job_id, error=str(exc))


def _elapsed_ms(started: float) -> int:
    return int((time.monotonic() - started) * 1000)
