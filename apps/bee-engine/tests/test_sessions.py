import asyncio

from bee_engine.sessions import BrowserSessionStore


class FakePage:
    def __init__(self) -> None:
        self.url = "about:blank"
        self.counter = 0

    async def goto(self, url, **_kwargs):
        self.url = url

    async def evaluate(self, code):
        if code == "window.counter = (window.counter || 0) + 1":
            self.counter += 1
        return self.counter

    async def screenshot(self, **_kwargs):
        return b"snapshot"

    async def title(self):
        return "Session page"


class FakeContext:
    def __init__(self) -> None:
        self.page = FakePage()
        self.closed = False

    async def new_page(self):
        return self.page

    async def route(self, *_args):
        return None

    async def close(self):
        self.closed = True


class FakeBrowser:
    def __init__(self) -> None:
        self.contexts = []

    async def new_context(self, **_kwargs):
        context = FakeContext()
        self.contexts.append(context)
        return context


class FakePool:
    def __init__(self) -> None:
        self.browser = FakeBrowser()

    async def _ensure_browser(self):
        return self.browser


def test_session_keeps_page_state_and_records_replay() -> None:
    async def scenario() -> None:
        pool = FakePool()
        store = BrowserSessionStore(pool, max_sessions=1)
        session = await store.create(
            ttl=60, activity_ttl=60, initial_url="https://example.com", storage_state=None
        )
        first = await store.execute(
            session.id,
            code="window.counter = (window.counter || 0) + 1",
            language="node",
            timeout=1,
        )
        second = await store.execute(
            session.id,
            code="window.counter = (window.counter || 0) + 1",
            language="node",
            timeout=1,
        )
        assert (first, second) == (1, 2)
        snapshots = await store.replay(session.id)
        assert len(snapshots) == 3
        assert (await store.replay_page(session.id, snapshots[-1].id)).url == "https://example.com"

        try:
            await store.create(ttl=60, activity_ttl=60)
        except RuntimeError as error:
            assert "concurrency" in str(error)
        else:
            raise AssertionError("session concurrency limit should be enforced")

        duration = await store.delete(session.id)
        assert duration >= 0
        assert pool.browser.contexts[0].closed

    asyncio.run(scenario())


def test_session_cleanup_expires_inactive_contexts() -> None:
    async def scenario() -> None:
        pool = FakePool()
        store = BrowserSessionStore(pool)
        session = await store.create(ttl=60, activity_ttl=10)
        session.last_activity -= 11
        assert await store.cleanup() == 1
        assert pool.browser.contexts[0].closed

    asyncio.run(scenario())
