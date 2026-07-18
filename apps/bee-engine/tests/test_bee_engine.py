import asyncio

from fastapi.testclient import TestClient

from bee_engine import app as app_module
from bee_engine.jobs import JobStore
from bee_engine.browser import _context_options, _uses_stealth
from bee_engine.models import BeeEngineScrapeRequest, BeeEngineScrapeResponse, ProxySettings


class FakeBrowserPool:
    async def render(self, request: BeeEngineScrapeRequest) -> BeeEngineScrapeResponse:
        action_results = []
        screenshots = []
        action_content = []
        for idx, action in enumerate(request.actions):
            if action.type == "screenshot":
                screenshots.append("fake-screenshot")
                action_results.append(
                    {"idx": idx, "type": "screenshot", "result": {"data": "fake-screenshot"}}
                )
            elif action.type == "executeJavascript":
                action_results.append(
                    {"idx": idx, "type": "executeJavascript", "result": {"return": "ok"}}
                )
            elif action.type == "scrape":
                action_content.append({"url": request.url, "html": "<html>action</html>"})
                action_results.append(
                    {
                        "idx": idx,
                        "type": "scrape",
                        "result": {"url": request.url, "html": "<html>action</html>"},
                    }
                )

        return BeeEngineScrapeResponse(
            timeTaken=0,
            content="<html><body><main>Rendered</main></body></html>",
            url=request.url,
            pageStatusCode=200,
            responseHeaders={"content-type": "text/html"},
            screenshots=screenshots,
            actionContent=action_content,
            actionResults=action_results,
        )

    async def close(self) -> None:
        return None

    def health(self) -> dict:
        return {
            "maxPages": 4,
            "availablePages": 3,
            "activePages": 1,
            "browserConnected": True,
        }


def test_scrape_sync_returns_fire_engine_style_response() -> None:
    app_module._browser_pool = FakeBrowserPool()
    app_module._job_store = JobStore(app_module._browser_pool)
    client = TestClient(app_module.app)

    response = client.post(
        "/scrape",
        json={
            "url": "https://example.com",
            "actions": [
                {"type": "screenshot", "fullPage": True},
                {"type": "executeJavascript", "script": "document.title"},
                {"type": "scrape"},
            ],
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert body["content"] == "<html><body><main>Rendered</main></body></html>"
    assert body["pageStatusCode"] == 200
    assert body["screenshots"] == ["fake-screenshot"]
    assert body["actionResults"][1]["type"] == "executeJavascript"
    assert body["actionContent"][0]["html"] == "<html>action</html>"


def test_health_reports_instance_capacity_engines_and_jobs() -> None:
    app_module._browser_pool = FakeBrowserPool()
    app_module._job_store = JobStore(app_module._browser_pool)
    body = TestClient(app_module.app).get("/health").json()
    assert body["ok"] is True
    assert body["capacity"]["availablePages"] == 3
    assert body["capacity"]["activePages"] == 1
    assert body["jobs"] == {"total": 0, "processing": 0, "completed": 0, "failed": 0}
    assert body["engines"]["playwright"] is True


def test_screenshot_action_accepts_quality_and_viewport() -> None:
    request = BeeEngineScrapeRequest.model_validate(
        {
            "url": "https://example.com",
            "actions": [
                {
                    "type": "screenshot",
                    "fullPage": True,
                    "quality": 80,
                    "viewport": {"width": 1440, "height": 900},
                }
            ],
        }
    )
    action = request.actions[0]
    assert action.type == "screenshot"
    assert action.quality == 80
    assert action.viewport.width == 1440


def test_proxy_settings_are_forwarded_to_browser_context() -> None:
    request = BeeEngineScrapeRequest.model_validate(
        {
            "url": "https://example.com",
            "proxy": {
                "mode": "basic",
                "server": "http://proxy.example.com:8080",
                "username": "bee",
                "password": "secret",
            },
        }
    )
    assert _context_options(request)["proxy"] == {
        "server": "http://proxy.example.com:8080",
        "username": "bee",
        "password": "secret",
    }
    assert not _uses_stealth(request)

    request.proxy.mode = "enhanced"
    assert _uses_stealth(request)


def test_fingerprint_proxy_url_preserves_encoded_credentials() -> None:
    proxy = ProxySettings(
        mode="stealth",
        server="http://proxy.example.com:8080/",
        username="bee user",
        password="p@ss",
    )
    assert (
        app_module._proxy_url(proxy)
        == "http://bee%20user:p%40ss@proxy.example.com:8080/"
    )


def test_scrape_instant_return_status_and_delete() -> None:
    app_module._browser_pool = FakeBrowserPool()
    app_module._job_store = JobStore(app_module._browser_pool)
    client = TestClient(app_module.app)

    response = client.post(
        "/scrape",
        json={"url": "https://example.com", "scrapeId": "job_test", "instantReturn": True},
    )

    assert response.status_code == 200
    assert response.json() == {"jobId": "job_test", "processing": True}

    async def wait_for_completion() -> None:
        for _ in range(20):
            status = await app_module._job_store.get("job_test")
            if status and getattr(status, "processing", False) is False:
                return
            if status and hasattr(status, "content"):
                return
            await asyncio.sleep(0.01)

    asyncio.run(wait_for_completion())

    status_response = client.get("/scrape/job_test")
    assert status_response.status_code == 200
    assert status_response.json()["content"] == "<html><body><main>Rendered</main></body></html>"

    delete_response = client.delete("/scrape/job_test")
    assert delete_response.status_code == 200
    assert client.get("/scrape/job_test").status_code == 404
