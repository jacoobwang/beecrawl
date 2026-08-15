from __future__ import annotations

import asyncio
import pytest

import httpx

from evals.scrape_benchmark import (
    _merge_request,
    _normalize_payload,
    _request_sample,
    _summarize_samples,
)


def test_merge_request_applies_track_overrides_without_mutating_case() -> None:
    suite = {
        "tracks": {
            "fresh": {"request_overrides": {"firecrawl": {"maxAge": 0}}},
        }
    }
    case = {
        "name": "example",
        "url": "https://example.com",
        "requests": {"firecrawl": {"formats": ["markdown"]}},
    }

    request = _merge_request(suite, "firecrawl", case, "fresh")

    assert request == {
        "url": "https://example.com",
        "formats": ["markdown"],
        "maxAge": 0,
    }
    assert case["requests"]["firecrawl"] == {"formats": ["markdown"]}


def test_normalize_payload_supports_firecrawl_envelope() -> None:
    normalized = _normalize_payload(
        {
            "success": True,
            "data": {
                "markdown": "# Example",
                "metadata": {"title": "Example", "provider": "bee_engine"},
            },
        }
    )

    assert normalized["markdown"] == "# Example"
    assert normalized["title"] == "Example"
    assert normalized["provider"] == "bee_engine"


def test_request_sample_records_quality_failure_without_raising() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            json={"markdown": "short", "title": "Example", "status": "success"},
            request=request,
        )

    case = {
        "name": "example",
        "url": "https://example.com",
        "assertions": {"min_markdown_chars": 100, "require_title": True},
    }

    async def run() -> dict:
        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
            return await _request_sample(
                client,
                "https://provider.test/scrape",
                {},
                {"url": case["url"]},
                case,
                1,
            )

    result = asyncio.run(run())

    assert result["response_status"] == 200
    assert result["quality_passed"] is False
    assert any(check["name"] == "minimum_markdown_length" and not check["passed"] for check in result["checks"])


def test_summarize_samples_reports_percentiles_and_throughput() -> None:
    samples = [
        {"elapsed_ms": 100, "quality_score": 1.0, "quality_passed": True, "error": None, "checks": [{"passed": True}]},
        {"elapsed_ms": 200, "quality_score": 0.5, "quality_passed": False, "error": "timeout", "checks": [{"passed": False}]},
        {"elapsed_ms": 300, "quality_score": 1.0, "quality_passed": True, "error": None, "checks": [{"passed": True}]},
    ]

    summary = _summarize_samples(samples, 600)

    assert summary["success_rate"] == pytest.approx(2 / 3, abs=0.0001)
    assert summary["quality_pass_rate"] == pytest.approx(2 / 3, abs=0.0001)
    assert summary["p50_elapsed_ms"] == 200
    assert summary["p95_elapsed_ms"] == 300
    assert summary["successful_pages_per_minute"] == 200.0
