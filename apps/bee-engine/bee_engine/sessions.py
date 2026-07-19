from __future__ import annotations

import asyncio
import base64
import json
import os
import time
import uuid
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import Any

from bee_engine.browser import BrowserPool, _route_handler


@dataclass
class BrowserSnapshot:
    id: str
    url: str
    title: str
    timestamp: float
    screenshot: str


@dataclass
class BrowserSession:
    id: str
    context: Any
    page: Any
    created_at: float
    expires_at: float
    activity_ttl: int
    last_activity: float
    record: bool
    snapshots: list[BrowserSnapshot] = field(default_factory=list)
    lock: asyncio.Lock = field(default_factory=asyncio.Lock)


class BrowserSessionStore:
    def __init__(self, browser_pool: BrowserPool, max_sessions: int | None = None) -> None:
        self._browser_pool = browser_pool
        self._max_sessions = max_sessions or int(os.getenv("BEE_ENGINE_MAX_SESSIONS", "8"))
        self._sessions: dict[str, BrowserSession] = {}
        self._lock = asyncio.Lock()

    async def create(
        self,
        *,
        ttl: int,
        activity_ttl: int,
        initial_url: str | None = None,
        storage_state: dict[str, Any] | None = None,
        record: bool = True,
    ) -> BrowserSession:
        await self.cleanup()
        async with self._lock:
            if len(self._sessions) >= self._max_sessions:
                raise RuntimeError("browser session concurrency limit reached")
            browser = await self._browser_pool._ensure_browser()
            context = await browser.new_context(storage_state=storage_state)
            await context.route("**/*", lambda route: _route_handler(route, block_media=False))
            page = await context.new_page()
            now = time.time()
            session = BrowserSession(
                id=uuid.uuid4().hex,
                context=context,
                page=page,
                created_at=now,
                expires_at=now + ttl,
                activity_ttl=activity_ttl,
                last_activity=now,
                record=record,
            )
            self._sessions[session.id] = session
        try:
            if initial_url:
                await page.goto(initial_url, wait_until="domcontentloaded", timeout=30000)
            await self._snapshot(session)
        except Exception:
            async with self._lock:
                self._sessions.pop(session.id, None)
            await context.close()
            raise
        return session

    async def list(self) -> list[BrowserSession]:
        await self.cleanup()
        async with self._lock:
            return list(self._sessions.values())

    async def health(self) -> dict[str, int]:
        await self.cleanup()
        async with self._lock:
            active = len(self._sessions)
        return {
            "active": active,
            "max": self._max_sessions,
            "available": self._max_sessions - active,
        }

    async def execute(
        self, session_id: str, *, code: str, language: str, timeout: int
    ) -> dict[str, Any] | None:
        session = await self.get(session_id)
        async with session.lock:
            session.last_activity = time.time()
            if language != "node":
                raise ValueError(
                    "Bee Engine browser sessions currently execute node/browser JavaScript"
                )
            result = await asyncio.wait_for(session.page.evaluate(code), timeout=timeout)
            await self._snapshot(session)
            return result

    async def get(self, session_id: str) -> BrowserSession:
        await self.cleanup()
        async with self._lock:
            session = self._sessions.get(session_id)
        if session is None:
            raise KeyError(session_id)
        return session

    async def delete(self, session_id: str) -> int:
        async with self._lock:
            session = self._sessions.pop(session_id, None)
        if session is None:
            raise KeyError(session_id)
        await session.context.close()
        return int((time.time() - session.created_at) * 1000)

    async def replay(self, session_id: str) -> list[BrowserSnapshot]:
        return list((await self.get(session_id)).snapshots)

    async def replay_page(self, session_id: str, page_id: str) -> BrowserSnapshot:
        for snapshot in await self.replay(session_id):
            if snapshot.id == page_id:
                return snapshot
        raise KeyError(page_id)

    async def cleanup(self) -> int:
        now = time.time()
        async with self._lock:
            expired = [
                session_id
                for session_id, session in self._sessions.items()
                if now >= session.expires_at or now - session.last_activity >= session.activity_ttl
            ]
            sessions = [self._sessions.pop(session_id) for session_id in expired]
        for session in sessions:
            await session.context.close()
        return len(sessions)

    async def close(self) -> None:
        async with self._lock:
            sessions = list(self._sessions.values())
            self._sessions.clear()
        for session in sessions:
            await session.context.close()

    async def _snapshot(self, session: BrowserSession) -> None:
        if not session.record:
            return
        screenshot = await session.page.screenshot(type="jpeg", quality=50)
        snapshot = BrowserSnapshot(
            id=uuid.uuid4().hex,
            url=session.page.url,
            title=await session.page.title(),
            timestamp=time.time(),
            screenshot="data:image/jpeg;base64," + base64.b64encode(screenshot).decode(),
        )
        session.snapshots.append(snapshot)
        del session.snapshots[:-50]


def session_json(session: BrowserSession) -> dict[str, Any]:
    return {
        "id": session.id,
        "status": "active",
        "createdAt": _iso(session.created_at),
        "lastActivity": _iso(session.last_activity),
        "expiresAt": _iso(session.expires_at),
        "url": session.page.url,
        "snapshotCount": len(session.snapshots),
    }


def snapshot_json(snapshot: BrowserSnapshot, include_image: bool = False) -> dict[str, Any]:
    data = {
        "id": snapshot.id,
        "url": snapshot.url,
        "title": snapshot.title,
        "timestamp": _iso(snapshot.timestamp),
    }
    if include_image:
        data["screenshot"] = snapshot.screenshot
    return data


def serialize_result(result: Any) -> str:
    try:
        return json.dumps(result)
    except TypeError:
        return str(result)


def _iso(timestamp: float) -> str:
    return datetime.fromtimestamp(timestamp, UTC).isoformat()
