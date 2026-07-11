import json

import httpx
import pytest

from beecrawl_sdk import BeeCrawlClient, BeeCrawlError


def test_client_requires_base_url():
    with pytest.raises(TypeError, match="base_url"):
        BeeCrawlClient()


def test_client_sends_auth_and_scrape_options():
    requests = []

    def handler(request: httpx.Request) -> httpx.Response:
        requests.append(request)
        return httpx.Response(200, json={"markdown": "hello"})

    transport = httpx.MockTransport(handler)
    with BeeCrawlClient(
        api_key="secret",
        base_url="http://api.test/",
        client=httpx.Client(transport=transport),
    ) as client:
        response = client.scrape("https://example.com", formats=["markdown", "links"])

    assert response == {"markdown": "hello"}
    assert requests[0].url == "http://api.test/scrape"
    assert requests[0].headers["x-web-extract-api-key"] == "secret"
    assert json.loads(requests[0].content) == {
        "url": "https://example.com",
        "formats": ["markdown", "links"],
    }


def test_client_parses_api_errors():
    def handler(_request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            401,
            json={"detail": {"code": "unauthorized", "message": "Invalid key"}},
        )

    with BeeCrawlClient(
        base_url="http://api.test",
        client=httpx.Client(transport=httpx.MockTransport(handler)),
    ) as client:
        with pytest.raises(BeeCrawlError, match="Invalid key") as error:
            client.map("https://example.com")

    assert error.value.status_code == 401
    assert error.value.detail["code"] == "unauthorized"


def test_poll_crawl_until_terminal_state():
    responses = iter(
        [
            {"id": "job-1", "status": "running"},
            {"id": "job-1", "status": "completed", "data": []},
        ]
    )

    with BeeCrawlClient(base_url="http://api.test") as client:
        client.crawl_status = lambda *_args, **_kwargs: next(responses)
        result = client.poll_crawl("job-1", interval=0, timeout=1)

    assert result["status"] == "completed"
