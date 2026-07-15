#!/usr/bin/env python3
"""Smoke-test BeeCrawl through the official Firecrawl Python v2 client."""

from __future__ import annotations

import argparse

from firecrawl import Firecrawl
from firecrawl.v2.utils.error_handler import (
    BadRequestError,
    FirecrawlError,
    WebsiteNotSupportedError,
)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--api-url", default="http://127.0.0.1:8000")
    args = parser.parse_args()
    client = Firecrawl(api_url=args.api_url, max_retries=0)
    blocked_url = "http://127.0.0.1:1/firecrawl-contract"

    try:
        client.scrape(blocked_url, formats=["markdown"])
    except WebsiteNotSupportedError as error:
        assert error.status_code == 403
    else:
        raise AssertionError("SDK-default scrape options did not reach BeeCrawl URL policy")

    try:
        client.scrape(blocked_url, formats=["markdown"], mobile=True)
    except BadRequestError as error:
        assert error.status_code == 400
        assert "mobile=true" in error.response.text
    else:
        raise AssertionError("unsupported Firecrawl option mobile=true was not rejected")

    try:
        client.start_crawl("https://example.com", formats=["markdown"])
    except FirecrawlError as error:
        assert error.status_code == 503

    print("official firecrawl-py v2 contract smoke passed")


if __name__ == "__main__":
    main()
