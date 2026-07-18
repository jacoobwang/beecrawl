#!/usr/bin/env python3
"""Smoke-test BeeCrawl through the official Firecrawl Python v2 client."""

from __future__ import annotations

import argparse
import tempfile
from pathlib import Path

from firecrawl import Firecrawl
from firecrawl.v2.utils.error_handler import (
    BadRequestError,
    FirecrawlError,
    WebsiteNotSupportedError,
)


def text_pdf() -> bytes:
    content = "BT /F1 12 Tf 72 720 Td (Hello BeeCrawl) Tj ET\n"
    document = (
        "%PDF-1.5\n"
        "1 0 obj<</Type/Pages/Kids[5 0 R]/Count 1/Resources 3 0 R/MediaBox[0 0 595 842]>>endobj\n"
        "2 0 obj<</Type/Font/Subtype/Type1/BaseFont/Courier>>endobj\n"
        "3 0 obj<</Font<</F1 2 0 R>>>>endobj\n"
        "5 0 obj<</Type/Page/Parent 1 0 R/Contents[4 0 R]>>endobj\n"
        "6 0 obj<</Type/Catalog/Pages 1 0 R>>endobj\n"
        f"4 0 obj<</Length {len(content)}>>stream\n{content}endstream\nendobj\n"
    )
    return (
        f"{document}xref\n0 7\n"
        "0000000000 65535 f \n0000000009 00000 n \n0000000096 00000 n \n"
        "0000000155 00000 n \n0000000291 00000 n \n0000000191 00000 n \n"
        "0000000248 00000 n \ntrailer\n<</Root 6 0 R/Size 7>>\n"
        f"startxref\n{len(document)}\n%%EOF"
    ).encode()


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
        client.map(blocked_url)
    except WebsiteNotSupportedError as error:
        assert error.status_code == 403
    else:
        raise AssertionError("map did not reach BeeCrawl URL policy")

    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "contract.pdf"
        path.write_bytes(text_pdf())
        parsed = client.parse(path)
        assert parsed.markdown == "Hello BeeCrawl"

    searched = client.search("beecrawl contract", sources=["news"])
    assert searched.news == []

    try:
        client.start_crawl("https://example.com", formats=["markdown"])
    except FirecrawlError as error:
        assert error.status_code == 503

    try:
        client.start_batch_scrape(["https://example.com"])
    except FirecrawlError as error:
        assert error.status_code == 503

    print("official firecrawl-py 4.32.1 v2 contract smoke passed")


if __name__ == "__main__":
    main()
