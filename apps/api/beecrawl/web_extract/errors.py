from __future__ import annotations

from dataclasses import dataclass


@dataclass(slots=True)
class WebExtractError(Exception):
    code: str
    message: str
    http_status: int
    retryable: bool

    def to_detail(self) -> dict[str, object]:
        return {
            "code": self.code,
            "message": self.message,
            "retryable": self.retryable,
        }


def invalid_url(message: str = "URL is invalid") -> WebExtractError:
    return WebExtractError("invalid_url", message, 400, False)


def blocked_by_policy(message: str = "URL is blocked by policy") -> WebExtractError:
    return WebExtractError("blocked_by_policy", message, 403, False)


def fetch_failed(message: str = "Failed to fetch URL") -> WebExtractError:
    return WebExtractError("fetch_failed", message, 502, True)


def render_timeout(message: str = "Browser render timed out") -> WebExtractError:
    return WebExtractError("render_timeout", message, 504, True)


def empty_content(message: str = "No usable page content") -> WebExtractError:
    return WebExtractError("empty_content", message, 422, False)
