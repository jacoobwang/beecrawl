"""Python HTTP client for the BeeCrawl API."""

from __future__ import annotations

import asyncio
import json
import time
from collections.abc import Mapping
from typing import Any

import httpx


class BeeCrawlError(Exception):
    """An API or transport error returned by BeeCrawl."""

    def __init__(self, message: str, *, status_code: int | None = None, detail: Any = None) -> None:
        super().__init__(message)
        self.message = message
        self.status_code = status_code
        self.detail = detail


class BeeCrawlClient:
    """Synchronous BeeCrawl API client.

    The client only talks to the BeeCrawl HTTP API. Browser rendering and
    crawling workers remain server-side.
    """

    def __init__(
        self,
        api_key: str | None = None,
        *,
        base_url: str,
        timeout: float = 60.0,
        client: httpx.Client | None = None,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self._client = client or httpx.Client(timeout=timeout)
        self._owns_client = client is None
        self._headers = {"X-Web-Extract-Api-Key": api_key} if api_key else {}

    def close(self) -> None:
        if self._owns_client:
            self._client.close()

    def __enter__(self) -> BeeCrawlClient:
        return self

    def __exit__(self, *_args: object) -> None:
        self.close()

    def scrape(self, url: str, **options: Any) -> dict[str, Any]:
        return self._post("/scrape", {"url": url, **options})

    def map(self, url: str, **options: Any) -> dict[str, Any]:
        return self._post("/map", {"url": url, **options})

    def search(self, query: str, **options: Any) -> dict[str, Any]:
        return self._post("/search", {"query": query, **options})

    def extract(self, url: str, schema: Mapping[str, str], **options: Any) -> dict[str, Any]:
        return self._post("/extract", {"url": url, "schema": dict(schema), **options})

    def crawl(self, url: str, **options: Any) -> dict[str, Any]:
        return self._post("/crawl", {"url": url, **options})

    def batch_scrape(self, urls: list[str], **options: Any) -> dict[str, Any]:
        return self._post("/batch/scrape", {"urls": urls, **options})

    def crawl_status(self, job_id: str, *, offset: int = 0, limit: int = 20) -> dict[str, Any]:
        return self._get(f"/crawl/{job_id}", params={"offset": offset, "limit": limit})

    def batch_scrape_status(
        self, job_id: str, *, offset: int = 0, limit: int = 20
    ) -> dict[str, Any]:
        return self._get(f"/batch/scrape/{job_id}", params={"offset": offset, "limit": limit})

    def cancel_crawl(self, job_id: str) -> dict[str, Any]:
        return self._delete(f"/crawl/{job_id}")

    def cancel_batch_scrape(self, job_id: str) -> dict[str, Any]:
        return self._delete(f"/batch/scrape/{job_id}")

    # Firecrawl-compatible v2 surface. Payloads intentionally stay as mappings
    # so new server options do not require an SDK release.
    def v2_scrape(self, url: str, **options: Any) -> dict[str, Any]:
        return self._post("/v2/scrape", {"url": url, **options})

    def v2_map(self, url: str, **options: Any) -> dict[str, Any]:
        return self._post("/v2/map", {"url": url, **options})

    def v2_search(self, query: str, **options: Any) -> dict[str, Any]:
        return self._post("/v2/search", {"query": query, **options})

    def v2_extract(self, urls: list[str], **options: Any) -> dict[str, Any]:
        return self._post("/v2/extract", {"urls": urls, **options})

    def v2_parse_base64(self, data: str, filename: str, **options: Any) -> dict[str, Any]:
        return self._post("/v2/parse/base64", {"base64": data, "filename": filename, **options})

    def v2_parse_reference(self, upload_ref: str, **options: Any) -> dict[str, Any]:
        return self._post("/v2/parse/reference", {"uploadRef": upload_ref, **options})

    def create_parse_upload(self, filename: str) -> dict[str, Any]:
        return self._post("/v2/parse/upload-url", {"filename": filename})

    def upload_parse_document(self, upload_ref: str, data: bytes) -> dict[str, Any]:
        return self._request("PUT", f"/v2/parse/upload/{upload_ref}", content=data)

    def v2_parse(self, filename: str, data: bytes, **options: Any) -> dict[str, Any]:
        return self._request(
            "POST", "/v2/parse", files={"file": (filename, data)}, data={"options": _json(options)}
        )

    def v2_crawl(self, url: str, **options: Any) -> dict[str, Any]:
        return self._post("/v2/crawl", {"url": url, **options})

    def v2_batch_scrape(self, urls: list[str], **options: Any) -> dict[str, Any]:
        return self._post("/v2/batch/scrape", {"urls": urls, **options})

    def v2_job_status(self, kind: str, job_id: str, **params: Any) -> dict[str, Any]:
        return self._get(f"/v2/{kind}/{job_id}", params=params)

    def v2_job_errors(self, kind: str, job_id: str) -> dict[str, Any]:
        return self._get(f"/v2/{kind}/{job_id}/errors")

    def cancel_v2_job(self, kind: str, job_id: str) -> dict[str, Any]:
        return self._delete(f"/v2/{kind}/{job_id}")

    def active_crawls(self) -> dict[str, Any]:
        return self._get("/v2/crawl/active")

    def create_browser_session(self, **options: Any) -> dict[str, Any]:
        return self._post("/v2/browser", options)

    def browser_sessions(self) -> dict[str, Any]:
        return self._get("/v2/browser")

    def execute_browser(self, session_id: str, code: str, **options: Any) -> dict[str, Any]:
        return self._post(f"/v2/browser/{session_id}/execute", {"code": code, **options})

    def browser_replay(self, session_id: str, page_id: str | None = None) -> dict[str, Any]:
        suffix = f"/{page_id}" if page_id else ""
        return self._get(f"/v2/browser/{session_id}/replay{suffix}")

    def delete_browser_session(self, session_id: str) -> dict[str, Any]:
        return self._delete(f"/v2/browser/{session_id}")

    def interact_with_scrape(self, scrape_id: str, **options: Any) -> dict[str, Any]:
        return self._post(f"/v2/scrape/{scrape_id}/interact", options)

    def delete_scrape_interaction(self, scrape_id: str) -> dict[str, Any]:
        return self._delete(f"/v2/scrape/{scrape_id}/interact")

    def create_agent(self, prompt: str, **options: Any) -> dict[str, Any]:
        return self._post("/v2/agent", {"prompt": prompt, **options})

    def get_agent(self, job_id: str) -> dict[str, Any]:
        return self._get(f"/v2/agent/{job_id}")

    def cancel_agent(self, job_id: str) -> dict[str, Any]:
        return self._delete(f"/v2/agent/{job_id}")

    def create_monitor(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._post("/v2/monitor", dict(payload))

    def list_monitors(self) -> dict[str, Any]:
        return self._get("/v2/monitor")

    def get_monitor(self, monitor_id: str) -> dict[str, Any]:
        return self._get(f"/v2/monitor/{monitor_id}")

    def update_monitor(self, monitor_id: str, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._request("PATCH", f"/v2/monitor/{monitor_id}", json=dict(payload))

    def delete_monitor(self, monitor_id: str) -> dict[str, Any]:
        return self._delete(f"/v2/monitor/{monitor_id}")

    def run_monitor(self, monitor_id: str) -> dict[str, Any]:
        return self._post(f"/v2/monitor/{monitor_id}/run", {})

    def monitor_checks(self, monitor_id: str, check_id: str | None = None) -> dict[str, Any]:
        suffix = f"/{check_id}" if check_id else ""
        return self._get(f"/v2/monitor/{monitor_id}/checks{suffix}")

    def poll_crawl(
        self,
        job_id: str,
        *,
        offset: int = 0,
        limit: int = 20,
        interval: float = 1.0,
        timeout: float = 300.0,
    ) -> dict[str, Any]:
        return self._poll(self.crawl_status, job_id, offset, limit, interval, timeout)

    def poll_batch_scrape(
        self,
        job_id: str,
        *,
        offset: int = 0,
        limit: int = 20,
        interval: float = 1.0,
        timeout: float = 300.0,
    ) -> dict[str, Any]:
        return self._poll(self.batch_scrape_status, job_id, offset, limit, interval, timeout)

    def _poll(
        self,
        status_method: Any,
        job_id: str,
        offset: int,
        limit: int,
        interval: float,
        timeout: float,
    ) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        while True:
            result = status_method(job_id, offset=offset, limit=limit)
            if result.get("status") in {"completed", "failed", "cancelled"}:
                return result
            if time.monotonic() >= deadline:
                raise BeeCrawlError(f"Timed out waiting for job {job_id}")
            time.sleep(interval)

    def _request(self, method: str, path: str, **kwargs: Any) -> dict[str, Any]:
        try:
            response = self._client.request(
                method, f"{self.base_url}{path}", headers=self._headers, **kwargs
            )
        except httpx.HTTPError as exc:
            raise BeeCrawlError(f"BeeCrawl request failed: {exc}") from exc
        if response.is_error:
            raise _error_from_response(response)
        try:
            payload = response.json()
        except ValueError as exc:
            raise BeeCrawlError(
                "BeeCrawl returned invalid JSON", status_code=response.status_code
            ) from exc
        if not isinstance(payload, dict):
            raise BeeCrawlError(
                "BeeCrawl returned a non-object JSON response", status_code=response.status_code
            )
        return payload

    def _post(self, path: str, payload: dict[str, Any]) -> dict[str, Any]:
        return self._request("POST", path, json=payload)

    def _get(self, path: str, *, params: dict[str, Any] | None = None) -> dict[str, Any]:
        return self._request("GET", path, params=params or {})

    def _delete(self, path: str) -> dict[str, Any]:
        return self._request("DELETE", path)


class AsyncBeeCrawlClient:
    """Asynchronous BeeCrawl API client with the same methods as the sync client."""

    def __init__(
        self,
        api_key: str | None = None,
        *,
        base_url: str,
        timeout: float = 60.0,
        client: httpx.AsyncClient | None = None,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self._client = client or httpx.AsyncClient(timeout=timeout)
        self._owns_client = client is None
        self._headers = {"X-Web-Extract-Api-Key": api_key} if api_key else {}

    async def close(self) -> None:
        if self._owns_client:
            await self._client.aclose()

    async def __aenter__(self) -> AsyncBeeCrawlClient:
        return self

    async def __aexit__(self, *_args: object) -> None:
        await self.close()

    async def scrape(self, url: str, **options: Any) -> dict[str, Any]:
        return await self._post("/scrape", {"url": url, **options})

    async def map(self, url: str, **options: Any) -> dict[str, Any]:
        return await self._post("/map", {"url": url, **options})

    async def search(self, query: str, **options: Any) -> dict[str, Any]:
        return await self._post("/search", {"query": query, **options})

    async def extract(self, url: str, schema: Mapping[str, str], **options: Any) -> dict[str, Any]:
        return await self._post("/extract", {"url": url, "schema": dict(schema), **options})

    async def crawl(self, url: str, **options: Any) -> dict[str, Any]:
        return await self._post("/crawl", {"url": url, **options})

    async def batch_scrape(self, urls: list[str], **options: Any) -> dict[str, Any]:
        return await self._post("/batch/scrape", {"urls": urls, **options})

    async def crawl_status(
        self, job_id: str, *, offset: int = 0, limit: int = 20
    ) -> dict[str, Any]:
        return await self._get(f"/crawl/{job_id}", params={"offset": offset, "limit": limit})

    async def batch_scrape_status(
        self, job_id: str, *, offset: int = 0, limit: int = 20
    ) -> dict[str, Any]:
        return await self._get(f"/batch/scrape/{job_id}", params={"offset": offset, "limit": limit})

    async def cancel_crawl(self, job_id: str) -> dict[str, Any]:
        return await self._delete(f"/crawl/{job_id}")

    async def cancel_batch_scrape(self, job_id: str) -> dict[str, Any]:
        return await self._delete(f"/batch/scrape/{job_id}")

    async def v2_scrape(self, url: str, **options: Any) -> dict[str, Any]:
        return await self._post("/v2/scrape", {"url": url, **options})

    async def v2_map(self, url: str, **options: Any) -> dict[str, Any]:
        return await self._post("/v2/map", {"url": url, **options})

    async def v2_search(self, query: str, **options: Any) -> dict[str, Any]:
        return await self._post("/v2/search", {"query": query, **options})

    async def v2_extract(self, urls: list[str], **options: Any) -> dict[str, Any]:
        return await self._post("/v2/extract", {"urls": urls, **options})

    async def v2_parse_base64(self, data: str, filename: str, **options: Any) -> dict[str, Any]:
        return await self._post(
            "/v2/parse/base64", {"base64": data, "filename": filename, **options}
        )

    async def v2_parse_reference(self, upload_ref: str, **options: Any) -> dict[str, Any]:
        return await self._post("/v2/parse/reference", {"uploadRef": upload_ref, **options})

    async def create_parse_upload(self, filename: str) -> dict[str, Any]:
        return await self._post("/v2/parse/upload-url", {"filename": filename})

    async def upload_parse_document(self, upload_ref: str, data: bytes) -> dict[str, Any]:
        return await self._request("PUT", f"/v2/parse/upload/{upload_ref}", content=data)

    async def v2_parse(self, filename: str, data: bytes, **options: Any) -> dict[str, Any]:
        return await self._request(
            "POST", "/v2/parse", files={"file": (filename, data)}, data={"options": _json(options)}
        )

    async def v2_crawl(self, url: str, **options: Any) -> dict[str, Any]:
        return await self._post("/v2/crawl", {"url": url, **options})

    async def v2_batch_scrape(self, urls: list[str], **options: Any) -> dict[str, Any]:
        return await self._post("/v2/batch/scrape", {"urls": urls, **options})

    async def v2_job_status(self, kind: str, job_id: str, **params: Any) -> dict[str, Any]:
        return await self._get(f"/v2/{kind}/{job_id}", params=params)

    async def v2_job_errors(self, kind: str, job_id: str) -> dict[str, Any]:
        return await self._get(f"/v2/{kind}/{job_id}/errors")

    async def cancel_v2_job(self, kind: str, job_id: str) -> dict[str, Any]:
        return await self._delete(f"/v2/{kind}/{job_id}")

    async def active_crawls(self) -> dict[str, Any]:
        return await self._get("/v2/crawl/active")

    async def create_browser_session(self, **options: Any) -> dict[str, Any]:
        return await self._post("/v2/browser", options)

    async def browser_sessions(self) -> dict[str, Any]:
        return await self._get("/v2/browser")

    async def execute_browser(self, session_id: str, code: str, **options: Any) -> dict[str, Any]:
        return await self._post(f"/v2/browser/{session_id}/execute", {"code": code, **options})

    async def browser_replay(self, session_id: str, page_id: str | None = None) -> dict[str, Any]:
        return await self._get(
            f"/v2/browser/{session_id}/replay" + (f"/{page_id}" if page_id else "")
        )

    async def delete_browser_session(self, session_id: str) -> dict[str, Any]:
        return await self._delete(f"/v2/browser/{session_id}")

    async def interact_with_scrape(self, scrape_id: str, **options: Any) -> dict[str, Any]:
        return await self._post(f"/v2/scrape/{scrape_id}/interact", options)

    async def delete_scrape_interaction(self, scrape_id: str) -> dict[str, Any]:
        return await self._delete(f"/v2/scrape/{scrape_id}/interact")

    async def create_agent(self, prompt: str, **options: Any) -> dict[str, Any]:
        return await self._post("/v2/agent", {"prompt": prompt, **options})

    async def get_agent(self, job_id: str) -> dict[str, Any]:
        return await self._get(f"/v2/agent/{job_id}")

    async def cancel_agent(self, job_id: str) -> dict[str, Any]:
        return await self._delete(f"/v2/agent/{job_id}")

    async def create_monitor(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return await self._post("/v2/monitor", dict(payload))

    async def list_monitors(self) -> dict[str, Any]:
        return await self._get("/v2/monitor")

    async def get_monitor(self, monitor_id: str) -> dict[str, Any]:
        return await self._get(f"/v2/monitor/{monitor_id}")

    async def update_monitor(self, monitor_id: str, payload: Mapping[str, Any]) -> dict[str, Any]:
        return await self._request("PATCH", f"/v2/monitor/{monitor_id}", json=dict(payload))

    async def delete_monitor(self, monitor_id: str) -> dict[str, Any]:
        return await self._delete(f"/v2/monitor/{monitor_id}")

    async def run_monitor(self, monitor_id: str) -> dict[str, Any]:
        return await self._post(f"/v2/monitor/{monitor_id}/run", {})

    async def monitor_checks(self, monitor_id: str, check_id: str | None = None) -> dict[str, Any]:
        return await self._get(
            f"/v2/monitor/{monitor_id}/checks" + (f"/{check_id}" if check_id else "")
        )

    async def poll_crawl(
        self,
        job_id: str,
        *,
        offset: int = 0,
        limit: int = 20,
        interval: float = 1.0,
        timeout: float = 300.0,
    ) -> dict[str, Any]:
        return await self._poll(self.crawl_status, job_id, offset, limit, interval, timeout)

    async def poll_batch_scrape(
        self,
        job_id: str,
        *,
        offset: int = 0,
        limit: int = 20,
        interval: float = 1.0,
        timeout: float = 300.0,
    ) -> dict[str, Any]:
        return await self._poll(self.batch_scrape_status, job_id, offset, limit, interval, timeout)

    async def _poll(
        self,
        status_method: Any,
        job_id: str,
        offset: int,
        limit: int,
        interval: float,
        timeout: float,
    ) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        while True:
            result = await status_method(job_id, offset=offset, limit=limit)
            if result.get("status") in {"completed", "failed", "cancelled"}:
                return result
            if time.monotonic() >= deadline:
                raise BeeCrawlError(f"Timed out waiting for job {job_id}")
            await asyncio.sleep(interval)

    async def _request(self, method: str, path: str, **kwargs: Any) -> dict[str, Any]:
        try:
            response = await self._client.request(
                method, f"{self.base_url}{path}", headers=self._headers, **kwargs
            )
        except httpx.HTTPError as exc:
            raise BeeCrawlError(f"BeeCrawl request failed: {exc}") from exc
        if response.is_error:
            raise _error_from_response(response)
        try:
            payload = response.json()
        except ValueError as exc:
            raise BeeCrawlError(
                "BeeCrawl returned invalid JSON", status_code=response.status_code
            ) from exc
        if not isinstance(payload, dict):
            raise BeeCrawlError(
                "BeeCrawl returned a non-object JSON response", status_code=response.status_code
            )
        return payload

    async def _post(self, path: str, payload: dict[str, Any]) -> dict[str, Any]:
        return await self._request("POST", path, json=payload)

    async def _get(self, path: str, *, params: dict[str, Any] | None = None) -> dict[str, Any]:
        return await self._request("GET", path, params=params or {})

    async def _delete(self, path: str) -> dict[str, Any]:
        return await self._request("DELETE", path)


def _error_from_response(response: httpx.Response) -> BeeCrawlError:
    try:
        payload = response.json()
    except ValueError:
        payload = response.text
    detail = payload.get("detail") if isinstance(payload, dict) else payload
    if isinstance(detail, dict):
        message = detail.get("message", "BeeCrawl request failed")
    else:
        message = str(detail or "BeeCrawl request failed")
    return BeeCrawlError(message, status_code=response.status_code, detail=detail)


def _json(value: Any) -> str:
    return json.dumps(value, separators=(",", ":"))


__all__ = ["AsyncBeeCrawlClient", "BeeCrawlClient", "BeeCrawlError"]
