from __future__ import annotations

import httpx

from evals.scrape_eval import build_report, evaluate_case, render_markdown


def _client(payload: dict, status_code: int = 200) -> httpx.Client:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(status_code, json=payload, request=request)

    return httpx.Client(transport=httpx.MockTransport(handler))


def test_evaluate_case_scores_observable_scrape_output() -> None:
    case = {
        "name": "article",
        "url": "https://example.com",
        "request": {"use_browser": "never"},
        "assertions": {
            "min_markdown_chars": 10,
            "required_text": ["Example"],
            "forbidden_text": ["captcha"],
            "require_title": True,
            "rendered": False,
            "provider_in": ["http_static"],
        },
    }
    payload = {
        "markdown": "# Example\n\nUseful content",
        "metadata": {
            "title": "Example",
            "provider": "http_static",
            "rendered": False,
        },
    }

    with _client(payload) as client:
        result = evaluate_case(client, "http://beecrawl.test", case)

    assert result["score"] == 1.0
    assert result["passed_checks"] == result["total_checks"] == 7


def test_build_report_fails_when_one_case_misses_the_case_floor() -> None:
    suite = {"name": "quality", "version": 1}
    baseline = {
        "minimum_overall_score": 0.8,
        "minimum_case_score": 0.85,
        "maximum_request_failures": 0,
    }
    results = [
        {
            "name": "good",
            "score": 1.0,
            "elapsed_ms": 100,
            "checks": [{"name": "request_succeeded", "passed": True}],
        },
        {
            "name": "weak",
            "score": 0.8,
            "elapsed_ms": 200,
            "checks": [{"name": "request_succeeded", "passed": True}],
        },
    ]

    report = build_report(suite, baseline, results, "http://beecrawl.test")

    assert report["summary"]["overall_score"] == 0.9
    assert report["passed"] is False
    assert report["gates"][1]["actual"] == {"failed_cases": ["weak"]}


def test_markdown_report_contains_stable_comment_marker() -> None:
    report = {
        "passed": True,
        "summary": {
            "overall_score": 1.0,
            "case_count": 1,
            "request_failures": 0,
            "p95_elapsed_ms": 120,
        },
        "cases": [
            {
                "name": "example",
                "score": 1.0,
                "elapsed_ms": 120,
                "provider": "http_static",
                "error": None,
                "checks": [{"name": "request_succeeded", "passed": True}],
            }
        ],
        "gates": [
            {
                "name": "minimum_overall_score",
                "passed": True,
                "expected": 0.95,
                "actual": 1.0,
            }
        ],
    }

    markdown = render_markdown(report)

    assert markdown.startswith("<!-- beecrawl-scrape-eval -->")
    assert "BeeCrawl scrape quality eval: PASS" in markdown
