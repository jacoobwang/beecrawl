"""Run BeeCrawl scrape quality evaluations against a live API."""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import httpx


def _check(name: str, passed: bool, expected: Any, actual: Any) -> dict[str, Any]:
    return {
        "name": name,
        "passed": passed,
        "expected": expected,
        "actual": actual,
    }


def evaluate_case(client: httpx.Client, api_url: str, case: dict[str, Any]) -> dict[str, Any]:
    request = {"url": case["url"], **case.get("request", {})}
    assertions = case.get("assertions", {})
    started = datetime.now(UTC)
    checks: list[dict[str, Any]] = []
    error: str | None = None
    payload: dict[str, Any] = {}
    response_status: int | None = None

    try:
        response = client.post(f"{api_url.rstrip('/')}/scrape", json=request)
        response_status = response.status_code
        checks.append(
            _check("request_succeeded", response.is_success, "HTTP 2xx", response.status_code)
        )
        if response.is_success:
            payload = response.json()
        else:
            error = response.text[:500]
    except Exception as exc:
        error = f"{type(exc).__name__}: {exc}"
        checks.append(_check("request_succeeded", False, "HTTP 2xx", error))

    elapsed_ms = round((datetime.now(UTC) - started).total_seconds() * 1000)
    if payload:
        markdown = payload.get("markdown")
        metadata = payload.get("metadata")
        markdown = markdown if isinstance(markdown, str) else ""
        metadata = metadata if isinstance(metadata, dict) else {}
        normalized_markdown = markdown.casefold()

        minimum_chars = assertions.get("min_markdown_chars")
        if minimum_chars is not None:
            checks.append(
                _check(
                    "minimum_markdown_length",
                    len(markdown) >= minimum_chars,
                    f">= {minimum_chars}",
                    len(markdown),
                )
            )

        required_text = assertions.get("required_text", [])
        if required_text:
            missing = [text for text in required_text if text.casefold() not in normalized_markdown]
            checks.append(_check("required_text", not missing, required_text, {"missing": missing}))

        forbidden_text = assertions.get("forbidden_text", [])
        if forbidden_text:
            found = [text for text in forbidden_text if text.casefold() in normalized_markdown]
            checks.append(_check("forbidden_text", not found, forbidden_text, {"found": found}))

        if assertions.get("require_title"):
            title = metadata.get("title")
            checks.append(
                _check(
                    "title_present",
                    bool(isinstance(title, str) and title.strip()),
                    True,
                    title,
                )
            )

        if "rendered" in assertions:
            checks.append(
                _check(
                    "rendered",
                    metadata.get("rendered") is assertions["rendered"],
                    assertions["rendered"],
                    metadata.get("rendered"),
                )
            )

        providers = assertions.get("provider_in", [])
        if providers:
            checks.append(
                _check(
                    "provider",
                    metadata.get("provider") in providers,
                    providers,
                    metadata.get("provider"),
                )
            )

    passed_checks = sum(1 for check in checks if check["passed"])
    score = passed_checks / len(checks) if checks else 0.0
    metadata = payload.get("metadata") if isinstance(payload.get("metadata"), dict) else {}
    return {
        "name": case["name"],
        "description": case.get("description"),
        "url": case["url"],
        "score": round(score, 4),
        "passed_checks": passed_checks,
        "total_checks": len(checks),
        "elapsed_ms": elapsed_ms,
        "response_status": response_status,
        "provider": metadata.get("provider"),
        "rendered": metadata.get("rendered"),
        "error": error,
        "checks": checks,
    }


def _percentile(values: list[int], percentile: float) -> int | None:
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, math.ceil(percentile * len(ordered)) - 1)
    return ordered[index]


def build_report(
    suite: dict[str, Any],
    baseline: dict[str, Any],
    results: list[dict[str, Any]],
    api_url: str,
) -> dict[str, Any]:
    overall_score = sum(result["score"] for result in results) / len(results) if results else 0.0
    request_failures = sum(
        1
        for result in results
        if not any(
            check["name"] == "request_succeeded" and check["passed"] for check in result["checks"]
        )
    )
    minimum_case_score = baseline["minimum_case_score"]
    failed_cases = [result["name"] for result in results if result["score"] < minimum_case_score]
    gates = [
        _check(
            "minimum_overall_score",
            overall_score >= baseline["minimum_overall_score"],
            baseline["minimum_overall_score"],
            round(overall_score, 4),
        ),
        _check(
            "minimum_case_score",
            not failed_cases,
            minimum_case_score,
            {"failed_cases": failed_cases},
        ),
        _check(
            "maximum_request_failures",
            request_failures <= baseline["maximum_request_failures"],
            baseline["maximum_request_failures"],
            request_failures,
        ),
    ]
    return {
        "suite": suite["name"],
        "suite_version": suite.get("version", 1),
        "generated_at": datetime.now(UTC).isoformat(),
        "api_url": api_url,
        "passed": all(gate["passed"] for gate in gates),
        "summary": {
            "overall_score": round(overall_score, 4),
            "case_count": len(results),
            "request_failures": request_failures,
            "p95_elapsed_ms": _percentile([result["elapsed_ms"] for result in results], 0.95),
        },
        "baseline": baseline,
        "gates": gates,
        "cases": results,
    }


def render_markdown(report: dict[str, Any]) -> str:
    status = "PASS" if report["passed"] else "FAIL"
    summary = report["summary"]
    lines = [
        "<!-- beecrawl-scrape-eval -->",
        f"## BeeCrawl scrape quality eval: {status}",
        "",
        f"Overall score: **{summary['overall_score']:.1%}** · "
        f"Cases: **{summary['case_count']}** · "
        f"Request failures: **{summary['request_failures']}** · "
        f"p95: **{summary['p95_elapsed_ms']} ms**",
        "",
        "| Case | Score | Time | Provider | Failed checks |",
        "| --- | ---: | ---: | --- | --- |",
    ]
    for result in report["cases"]:
        failed = ", ".join(check["name"] for check in result["checks"] if not check["passed"])
        if result["error"]:
            failed = f"{failed}; {result['error']}" if failed else result["error"]
        failed = failed.replace("\n", " ").replace("|", "\\|") if failed else "—"
        provider = (result["provider"] or "—").replace("|", "\\|")
        lines.append(
            f"| {result['name']} | {result['score']:.1%} | "
            f"{result['elapsed_ms']} ms | {provider} | {failed} |"
        )
    lines.extend(["", "### Gates", ""])
    for gate in report["gates"]:
        marker = "✅" if gate["passed"] else "❌"
        lines.append(
            f"- {marker} `{gate['name']}`: expected `{gate['expected']}`, actual `{gate['actual']}`"
        )
    return "\n".join(lines) + "\n"


def load_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as file:
        value = json.load(file)
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api-url", default="http://127.0.0.1:8000")
    parser.add_argument(
        "--suite",
        type=Path,
        default=Path("evals/scrape-quality/cases.json"),
    )
    parser.add_argument(
        "--baseline",
        type=Path,
        default=Path("evals/scrape-quality/baseline.json"),
    )
    parser.add_argument("--output-dir", type=Path, default=Path("artifacts/scrape-evals"))
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    suite = load_json(args.suite)
    baseline = load_json(args.baseline)
    timeout = httpx.Timeout(40.0, connect=10.0)
    with httpx.Client(timeout=timeout) as client:
        results = [evaluate_case(client, args.api_url, case) for case in suite.get("cases", [])]
    report = build_report(suite, baseline, results, args.api_url)
    markdown = render_markdown(report)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "report.json").write_text(
        json.dumps(report, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    (args.output_dir / "summary.md").write_text(markdown, encoding="utf-8")
    print(markdown)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
