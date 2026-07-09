from __future__ import annotations

import re
from urllib.parse import urljoin

from bs4 import BeautifulSoup

from beecrawl.models import Link, ScrapeResponse
from beecrawl.web_extract.providers.http_static import extract_markdown

EMAIL_RE = re.compile(r"[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}", re.IGNORECASE)
PHONE_RE = re.compile(r"(?:\+?\d[\d\s().-]{7,}\d)")


def parse_html(url: str, html: str) -> ScrapeResponse:
    soup = BeautifulSoup(html, "html.parser")

    for tag in soup(["script", "style", "noscript", "template"]):
        tag.decompose()

    title = _clean_text(soup.title.get_text(" ")) if soup.title else None
    text, markdown_metadata = extract_markdown(html, url)
    if not text:
        text = _extract_text(soup)
    links = _extract_links(url, soup)
    metadata = _extract_metadata(soup)
    metadata.update({key: value for key, value in markdown_metadata.items() if value})

    return ScrapeResponse(
        url=url,
        title=title,
        text=text,
        links=links,
        metadata=metadata,
    )


def extract_fields(scrape: ScrapeResponse, schema: dict[str, str]) -> dict[str, str | None]:
    data: dict[str, str | None] = {}
    haystack = "\n".join([scrape.title or "", scrape.text])

    for field_name in schema:
        normalized = field_name.lower()
        if "email" in normalized:
            data[field_name] = _first_match(EMAIL_RE, haystack)
        elif "phone" in normalized or "tel" in normalized:
            data[field_name] = _first_match(PHONE_RE, haystack)
        elif "title" in normalized or "name" in normalized:
            data[field_name] = scrape.title
        else:
            data[field_name] = None

    return data


def _extract_text(soup: BeautifulSoup) -> str:
    blocks = []
    for element in soup.find_all(["h1", "h2", "h3", "p", "li"]):
        value = _clean_text(element.get_text(" "))
        if value:
            blocks.append(value)
    return "\n".join(dict.fromkeys(blocks))


def _extract_links(base_url: str, soup: BeautifulSoup) -> list[Link]:
    links: list[Link] = []
    seen: set[str] = set()

    for anchor in soup.find_all("a", href=True):
        href = urljoin(base_url, str(anchor["href"]))
        text = _clean_text(anchor.get_text(" "))
        if href in seen:
            continue
        seen.add(href)
        links.append(Link(text=text, url=href))

    return links


def _extract_metadata(soup: BeautifulSoup) -> dict[str, str]:
    metadata: dict[str, str] = {}
    for meta in soup.find_all("meta"):
        key = meta.get("name") or meta.get("property")
        content = meta.get("content")
        if key and content:
            metadata[str(key)] = _clean_text(str(content))
    return metadata


def _clean_text(value: str) -> str:
    return " ".join(value.split())


def _first_match(pattern: re.Pattern[str], value: str) -> str | None:
    match = pattern.search(value)
    return match.group(0) if match else None
