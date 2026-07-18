from __future__ import annotations

import asyncio
import base64
import os
import socket
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from contextlib import suppress

from fastapi import FastAPI, HTTPException

from bee_engine.browser import BrowserPool
from bee_engine.jobs import JobStore
from bee_engine.models import (
    BeeEngineScrapeRequest,
    BeeEngineScrapeResponse,
    BeeEngineStatusResponse,
    BrowserSessionCreateRequest,
    BrowserSessionExecuteRequest,
    FingerprintFetchRequest,
    FingerprintFetchResponse,
    DocumentParseRequest,
    DocumentParseResponse,
    ProcessingResponse,
)
from bee_engine.sessions import BrowserSessionStore, serialize_result, session_json, snapshot_json

_browser_pool = BrowserPool()
_job_store = JobStore(_browser_pool)
_session_store = BrowserSessionStore(_browser_pool)


@asynccontextmanager
async def lifespan(_app: FastAPI) -> AsyncIterator[None]:
    cleanup_task = asyncio.create_task(_session_cleanup_loop())
    try:
        yield
    finally:
        cleanup_task.cancel()
        with suppress(asyncio.CancelledError):
            await cleanup_task
        await _session_store.close()
        await _browser_pool.close()


async def _session_cleanup_loop() -> None:
    while True:
        await asyncio.sleep(30)
        await _session_store.cleanup()


app = FastAPI(
    title="Bee Engine",
    description="Browser rendering service for Beecrawl.",
    version="0.1.0",
    lifespan=lifespan,
)


@app.get("/health")
async def health() -> dict:
    capacity = _browser_pool.health()
    return {
        "ok": True,
        "instanceId": os.getenv("BEE_ENGINE_INSTANCE_ID", socket.gethostname()),
        "version": app.version,
        "capacity": capacity,
        "jobs": await _job_store.health(),
        "sessions": await _session_store.health(),
        "engines": {
            "playwright": True,
            "chromeCdp": True,
            "tlsFingerprint": _fingerprint_available(),
        },
    }


def _fingerprint_available() -> bool:
    try:
        import curl_cffi  # noqa: F401
    except ImportError:
        return False
    return True


@app.post("/parse", response_model=DocumentParseResponse)
async def parse_document_endpoint(request: DocumentParseRequest) -> DocumentParseResponse:
    from bee_engine.document_parser import parse_document, rendered_formats

    try:
        data = base64.b64decode(request.base64, validate=True)
        markdown, metadata = await asyncio.to_thread(
            parse_document,
            data,
            request.filename,
            mode=request.mode,
            max_pages=request.max_pages,
        )
    except Exception as exc:
        raise HTTPException(status_code=422, detail=f"Document parsing failed: {exc}") from exc
    return DocumentParseResponse(
        data=rendered_formats(markdown, request.formats),
        metadata=metadata,
    )


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


@app.post("/sessions")
async def create_session(request: BrowserSessionCreateRequest) -> dict:
    try:
        session = await _session_store.create(
            ttl=request.ttl,
            activity_ttl=request.activity_ttl,
            initial_url=request.initial_url,
            storage_state=request.storage_state,
            record=request.record,
        )
    except RuntimeError as exc:
        raise HTTPException(status_code=429, detail=str(exc)) from exc
    except Exception as exc:
        raise HTTPException(
            status_code=502, detail=f"Could not create browser session: {exc}"
        ) from exc
    return session_json(session)


@app.get("/sessions")
async def list_sessions() -> dict:
    return {"sessions": [session_json(session) for session in await _session_store.list()]}


@app.post("/sessions/{session_id}/execute")
async def execute_session(session_id: str, request: BrowserSessionExecuteRequest) -> dict:
    try:
        result = await _session_store.execute(
            session_id, code=request.code, language=request.language, timeout=request.timeout
        )
    except KeyError as exc:
        raise HTTPException(status_code=404, detail="Browser session not found") from exc
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    except TimeoutError as exc:
        raise HTTPException(status_code=408, detail="Browser execution timed out") from exc
    except Exception as exc:
        raise HTTPException(status_code=502, detail=f"Browser execution failed: {exc}") from exc
    return {
        "stdout": "",
        "result": serialize_result(result),
        "stderr": "",
        "exitCode": 0,
        "killed": False,
    }


@app.get("/sessions/{session_id}/replay")
async def session_replay(session_id: str) -> dict:
    try:
        snapshots = await _session_store.replay(session_id)
    except KeyError as exc:
        raise HTTPException(status_code=404, detail="Browser session not found") from exc
    return {"pages": [snapshot_json(snapshot) for snapshot in snapshots]}


@app.get("/sessions/{session_id}/replay/{page_id}")
async def session_replay_page(session_id: str, page_id: str) -> dict:
    try:
        return snapshot_json(await _session_store.replay_page(session_id, page_id), True)
    except KeyError as exc:
        raise HTTPException(status_code=404, detail="Browser replay page not found") from exc


@app.delete("/sessions/{session_id}")
async def delete_session(session_id: str) -> dict:
    try:
        duration = await _session_store.delete(session_id)
    except KeyError as exc:
        raise HTTPException(status_code=404, detail="Browser session not found") from exc
    return {"ok": True, "sessionDurationMs": duration}


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
