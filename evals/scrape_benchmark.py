"""Run a reproducible multi-provider web extraction benchmark."""

from __future__ import annotations

import argparse
import asyncio
import json
import math
import os
import sys
import time
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


def _percentile(values: list[int], percentile: float) -> int | None:
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, math.ceil(percentile * len(ordered)) - 1)
    return ordered[index]


def _merge_request(
    suite: dict[str, Any], provider: str, case: dict[str, Any], track: str
) -> dict[str, Any]:
    requests = case.get("requests", {})
    if provider not in requests:
        raise ValueError(f"case {case['name']!r} has no request for provider {provider!r}")
    request = {"url": case["url"], **requests[provider]}
    track_config = suite.get("tracks", {}).get(track)
    if not isinstance(track_config, dict):
        raise ValueError(f"unknown benchmark track: {track}")
    overrides = track_config.get("request_overrides", {}).get(provider, {})
    if not isinstance(overrides, dict):
        raise ValueError(f"track {track!r} has invalid overrides for {provider!r}")
    request.update(overrides)
    return request


def _auth_headers(provider_config: dict[str, Any]) -> dict[str, str]:
    env_name = provider_config.get("api_key_env")
    if not env_name:
        return {}
    api_key = os.environ.get(env_name)
    if not api_key:
        return {}
    auth = provider_config.get("auth", "bearer")
    if auth == "bearer":
        return {"Authorization": f"Bearer {api_key}"}
    if auth == "x-api-key":
        return {"X-Api-Key": api_key}
    raise ValueError(f"unsupported auth mode: {auth}")


def _normalize_payload(payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict):
        return {"markdown": "", "metadata": {}, "status": None}
    data = payload.get("data") if isinstance(payload.get("data"), dict) else payload
    metadata = data.get("metadata") if isinstance(data.get("metadata"), dict) else {}
    title = metadata.get("title") or data.get("title") or payload.get("title")
    provider = metadata.get("provider") or data.get("provider") or payload.get("provider")
    rendered = metadata.get("rendered")
    if rendered is None:
        rendered = data.get("rendered")
    return {
        "markdown": data.get("markdown") if isinstance(data.get("markdown"), str) else "",
        "metadata": metadata,
        "title": title,
        "provider": provider,
        "rendered": rendered,
        "status": data.get("status") or payload.get("status"),
        "success": payload.get("success"),
    }


def _evaluate_sample(
    case: dict[str, Any], payload: Any, response_succeeded: bool, response_error: str | None
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    normalized = _normalize_payload(payload)
    semantic_success = response_succeeded and normalized["status"] != "error" and normalized["success"] is not False
    checks = [_check("request_succeeded", semantic_success, "HTTP 2xx and successful payload", response_error or semantic_success)]
    assertions = case.get("assertions", {})
    markdown = normalized["markdown"]
    normalized_markdown = markdown.casefold()

    if semantic_success:
        minimum_chars = assertions.get("min_markdown_chars")
        if minimum_chars is not None:
            checks.append(
                _check("minimum_markdown_length", len(markdown) >= minimum_chars, f">= {minimum_chars}", len(markdown))
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
            checks.append(
                _check(
                    "title_present",
                    isinstance(normalized["title"], str) and bool(normalized["title"].strip()),
                    True,
                    normalized["title"],
                )
            )

        if "rendered" in assertions:
            checks.append(_check("rendered", normalized["rendered"] is assertions["rendered"], assertions["rendered"], normalized["rendered"]))

        providers = assertions.get("provider_in", [])
        if providers:
            checks.append(_check("provider", normalized["provider"] in providers, providers, normalized["provider"]))

    score = sum(check["passed"] for check in checks) / len(checks) if checks else 0.0
    return checks, {
        "markdown_chars": len(markdown),
        "title": normalized["title"],
        "provider": normalized["provider"],
        "rendered": normalized["rendered"],
        "quality_score": round(score, 4),
        "quality_passed": bool(checks) and all(check["passed"] for check in checks),
    }


async def _request_sample(
    client: httpx.AsyncClient,
    endpoint: str,
    headers: dict[str, str],
    request: dict[str, Any],
    case: dict[str, Any],
    sample_number: int,
) -> dict[str, Any]:
    started = time.perf_counter()
    payload: Any = None
    response_status: int | None = None
    error: str | None = None
    try:
        response = await client.post(endpoint, headers=headers, json=request)
        response_status = response.status_code
        if response.is_success:
            try:
                payload = response.json()
            except ValueError as exc:
                error = f"invalid JSON response: {exc}"
        else:
            error = response.text[:500]
    except Exception as exc:  # noqa: BLE001 - benchmark records provider failures as data
        error = f"{type(exc).__name__}: {exc}"
    elapsed_ms = round((time.perf_counter() - started) * 1000)
    checks, output = _evaluate_sample(case, payload, response_status is not None and 200 <= response_status < 300 and error is None, error)
    return {
        "sample": sample_number,
        "elapsed_ms": elapsed_ms,
        "response_status": response_status,
        "error": error,
        **output,
        "checks": checks,
    }


async def _run_batch(
    client: httpx.AsyncClient,
    endpoint: str,
    headers: dict[str, str],
    request: dict[str, Any],
    case: dict[str, Any],
    count: int,
    concurrency: int,
) -> tuple[list[dict[str, Any]], int]:
    semaphore = asyncio.Semaphore(concurrency)

    async def run_one(sample_number: int) -> dict[str, Any]:
        async with semaphore:
            return await _request_sample(client, endpoint, headers, request, case, sample_number)

    started = time.perf_counter()
    results = await asyncio.gather(*(run_one(number) for number in range(1, count + 1)))
    elapsed_ms = round((time.perf_counter() - started) * 1000)
    return results, elapsed_ms


def _summarize_samples(samples: list[dict[str, Any]], batch_elapsed_ms: int) -> dict[str, Any]:
    successes = [sample for sample in samples if sample["checks"][0]["passed"]]
    latencies = [sample["elapsed_ms"] for sample in samples]
    quality_scores = [sample["quality_score"] for sample in samples]
    return {
        "sample_count": len(samples),
        "success_count": len(successes),
        "success_rate": round(len(successes) / len(samples), 4) if samples else 0.0,
        "quality_pass_count": sum(sample["quality_passed"] for sample in samples),
        "quality_pass_rate": round(sum(sample["quality_passed"] for sample in samples) / len(samples), 4) if samples else 0.0,
        "average_quality_score": round(sum(quality_scores) / len(quality_scores), 4) if quality_scores else 0.0,
        "p50_elapsed_ms": _percentile(latencies, 0.50),
        "p95_elapsed_ms": _percentile(latencies, 0.95),
        "p99_elapsed_ms": _percentile(latencies, 0.99),
        "batch_elapsed_ms": batch_elapsed_ms,
        "successful_pages_per_minute": round(len(successes) * 60_000 / batch_elapsed_ms, 2) if batch_elapsed_ms else 0.0,
        "error_count": sum(sample["error"] is not None for sample in samples),
    }


async def run_benchmark(
    suite: dict[str, Any],
    providers: dict[str, str],
    track: str,
    samples: int,
    warmup: int,
    concurrency: int,
    timeout_seconds: float,
    region: str,
    price_per_1000_request: dict[str, float],
) -> dict[str, Any]:
    cases: list[dict[str, Any]] = []
    raw_samples: list[dict[str, Any]] = []
    limits = httpx.Limits(max_connections=max(1, concurrency), max_keepalive_connections=max(1, concurrency))
    timeout = httpx.Timeout(timeout_seconds, connect=min(timeout_seconds, 10.0))
    async with httpx.AsyncClient(timeout=timeout, limits=limits) as client:
        for provider, base_url in providers.items():
            provider_config = suite["providers"][provider]
            endpoint = f"{base_url.rstrip('/')}{provider_config['path']}"
            headers = _auth_headers(provider_config)
            for case in suite.get("cases", []):
                request = _merge_request(suite, provider, case, track)
                if warmup:
                    await _run_batch(client, endpoint, headers, request, case, warmup, concurrency)
                measured, batch_elapsed_ms = await _run_batch(
                    client, endpoint, headers, request, case, samples, concurrency
                )
                summary = _summarize_samples(measured, batch_elapsed_ms)
                cases.append({"provider": provider, "case": case["name"], "category": case.get("category"), **summary})
                for sample in measured:
                    raw_samples.append({
                        "provider": provider,
                        "case": case["name"],
                        "category": case.get("category"),
                        "track": track,
                        **sample,
                    })

    summaries = []
    for provider in providers:
        provider_cases = [case for case in cases if case["provider"] == provider]
        total_samples = sum(case["sample_count"] for case in provider_cases)
        total_successes = sum(case["success_count"] for case in provider_cases)
        total_quality_passes = sum(case["quality_pass_count"] for case in provider_cases)
        summaries.append({
            "provider": provider,
            "case_count": len(provider_cases),
            "sample_count": total_samples,
            "success_rate": round(total_successes / total_samples, 4) if total_samples else 0.0,
            "quality_pass_rate": round(total_quality_passes / total_samples, 4) if total_samples else 0.0,
            "p50_elapsed_ms": _percentile([sample["elapsed_ms"] for sample in raw_samples if sample["provider"] == provider], 0.50),
            "p95_elapsed_ms": _percentile([sample["elapsed_ms"] for sample in raw_samples if sample["provider"] == provider], 0.95),
            "p99_elapsed_ms": _percentile([sample["elapsed_ms"] for sample in raw_samples if sample["provider"] == provider], 0.99),
            "error_count": sum(case["error_count"] for case in provider_cases),
            "estimated_cost_usd_per_1000_successful_pages": (
                round(
                    total_samples * price_per_1000_request[provider] / total_successes,
                    4,
                )
                if provider in price_per_1000_request and total_successes
                else None
            ),
        })

    return {
        "benchmark": suite["name"],
        "version": suite.get("version", 1),
        "generated_at": datetime.now(UTC).isoformat(),
        "track": track,
        "parameters": {
            "samples": samples,
            "warmup": warmup,
            "concurrency": concurrency,
            "timeout_seconds": timeout_seconds,
            "region": region,
            "price_per_1000_request": price_per_1000_request,
        },
        "providers": providers,
        "configuration": {
            "track": suite["tracks"][track],
            "cases": [
                {
                    "name": case["name"],
                    "category": case.get("category"),
                    "url": case["url"],
                    "requests": case.get("requests", {}),
                    "assertions": case.get("assertions", {}),
                }
                for case in suite.get("cases", [])
            ],
        },
        "summary": summaries,
        "cases": cases,
        "samples": raw_samples,
    }


def render_markdown(report: dict[str, Any]) -> str:
    parameters = report["parameters"]
    lines = [
        "<!-- beecrawl-scrape-benchmark -->",
        "# BeeCrawl web extraction benchmark",
        "",
        f"Track: **{report['track']}** · Samples: **{parameters['samples']}** · "
        f"Warmups: **{parameters['warmup']}** · Concurrency: **{parameters['concurrency']}** · "
        f"Region: **{parameters['region']}**",
        "",
        "| Provider | Success | Quality | p50 | p95 | p99 | Cost/1k | Errors |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for summary in report["summary"]:
        cost = summary["estimated_cost_usd_per_1000_successful_pages"]
        lines.append(
            f"| {summary['provider']} | {summary['success_rate']:.1%} | "
            f"{summary['quality_pass_rate']:.1%} | {summary['p50_elapsed_ms']} ms | "
            f"{summary['p95_elapsed_ms']} ms | {summary['p99_elapsed_ms']} ms | "
            f"{cost if cost is not None else '—'} | {summary['error_count']} |"
        )
    lines.extend(["", "## Cases", "", "| Provider | Case | Samples | Success | Quality | p95 | Pages/min |", "| --- | --- | ---: | ---: | ---: | ---: | ---: |"])
    for case in report["cases"]:
        lines.append(
            f"| {case['provider']} | {case['case']} | {case['sample_count']} | "
            f"{case['success_rate']:.1%} | {case['quality_pass_rate']:.1%} | "
            f"{case['p95_elapsed_ms']} ms | {case['successful_pages_per_minute']} |"
        )
    lines.extend([
        "",
        "This report is a snapshot. Publish the suite, provider versions, request configuration, region, and raw `samples.jsonl` together with any public comparison.",
        "",
    ])
    return "\n".join(lines)


def load_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as file:
        value = json.load(file)
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def parse_provider(value: str, suite: dict[str, Any]) -> tuple[str, str]:
    name, separator, base_url = value.partition("=")
    if not separator or not base_url:
        raise argparse.ArgumentTypeError("provider must use NAME=BASE_URL")
    if name not in suite.get("providers", {}):
        known = ", ".join(suite.get("providers", {}))
        raise argparse.ArgumentTypeError(f"unknown provider {name!r}; choose from {known}")
    return name, base_url.rstrip("/")


def parse_price(value: str, suite: dict[str, Any]) -> tuple[str, float]:
    name, separator, price = value.partition("=")
    if not separator or not price:
        raise argparse.ArgumentTypeError("price must use PROVIDER=USD_PER_1000_REQUESTS")
    if name not in suite.get("providers", {}):
        known = ", ".join(suite.get("providers", {}))
        raise argparse.ArgumentTypeError(f"unknown provider {name!r}; choose from {known}")
    try:
        parsed = float(price)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("price must be a number") from exc
    if parsed < 0:
        raise argparse.ArgumentTypeError("price must be non-negative")
    return name, parsed


def parse_args(suite: dict[str, Any]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--provider",
        action="append",
        required=True,
        type=lambda value: parse_provider(value, suite),
        metavar="NAME=BASE_URL",
    )
    parser.add_argument(
        "--price-per-1000-request",
        action="append",
        type=lambda value: parse_price(value, suite),
        default=[],
        metavar="PROVIDER=USD",
        help="optional effective provider price per 1,000 attempted requests",
    )
    parser.add_argument("--track", choices=sorted(suite.get("tracks", {})), default="fresh")
    parser.add_argument("--samples", type=int, default=20)
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("--concurrency", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=45.0, dest="timeout_seconds")
    parser.add_argument("--region", default=os.environ.get("BENCHMARK_REGION", "unspecified"))
    parser.add_argument("--suite", type=Path, default=Path("evals/scrape-quality/benchmark.json"))
    parser.add_argument("--output-dir", type=Path, default=Path("artifacts/scrape-benchmark"))
    return parser.parse_args()


def main() -> int:
    suite_path_parser = argparse.ArgumentParser(add_help=False)
    suite_path_parser.add_argument(
        "--suite", type=Path, default=Path("evals/scrape-quality/benchmark.json")
    )
    suite_path, _ = suite_path_parser.parse_known_args()
    suite = load_json(suite_path.suite)
    args = parse_args(suite)
    if args.samples < 1 or args.warmup < 0 or args.concurrency < 1 or args.timeout_seconds <= 0:
        raise SystemExit("samples must be >= 1, warmup >= 0, concurrency >= 1, and timeout > 0")
    providers = dict(args.provider)
    if len(providers) != len(args.provider):
        raise SystemExit("each provider may be specified only once")
    prices = dict(args.price_per_1000_request)
    report = asyncio.run(
        run_benchmark(
            suite,
            providers,
            args.track,
            args.samples,
            args.warmup,
            args.concurrency,
            args.timeout_seconds,
            args.region,
            prices,
        )
    )
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "report.json").write_text(
        json.dumps({key: value for key, value in report.items() if key != "samples"}, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    with (args.output_dir / "samples.jsonl").open("w", encoding="utf-8") as file:
        for sample in report["samples"]:
            file.write(json.dumps(sample, ensure_ascii=False) + "\n")
    markdown = render_markdown(report)
    (args.output_dir / "summary.md").write_text(markdown, encoding="utf-8")
    print(markdown)
    return 0 if any(summary["success_rate"] > 0 for summary in report["summary"]) else 1


if __name__ == "__main__":
    sys.exit(main())
