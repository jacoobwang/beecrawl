from __future__ import annotations

import ipaddress
import re
from urllib.parse import urldefrag, urljoin, urlparse
from xml.etree import ElementTree

import httpx
from bs4 import BeautifulSoup
from markdownify import markdownify

from beecrawl.models import ProviderPage
from beecrawl.web_extract.errors import blocked_by_policy, fetch_failed, invalid_url

USER_AGENT = (
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
    "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36"
)


def normalize_url(raw_url: str) -> str:
    value = str(raw_url or "").strip()
    if not value:
        raise invalid_url("URL is required")
    parsed = urlparse(value)
    if not parsed.scheme:
        value = f"https://{value}"
        parsed = urlparse(value)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise invalid_url("Only http and https URLs are supported")
    host = parsed.hostname or ""
    if host in {"localhost"} or host.endswith(".localhost"):
        raise blocked_by_policy("Localhost URLs are not allowed")
    try:
        ip = ipaddress.ip_address(host)
        if ip.is_private or ip.is_loopback or ip.is_link_local or ip.is_reserved:
            raise blocked_by_policy("Private network URLs are not allowed")
    except ValueError:
        pass
    return value


def extract_markdown(html: str, base_url: str) -> tuple[str, dict[str, str | None]]:
    soup = _clean_html(html, base_url)
    title = _text(soup.title.string if soup.title else "")
    language = (soup.html or {}).get("lang") if soup.html else None
    main = soup.find("main") or soup.find("article") or soup.body or soup

    markdown = markdownify(
        str(main),
        heading_style="ATX",
        bullets="-",
        strip=["script", "style", "noscript", "svg", "canvas", "iframe"],
    )
    markdown = _post_process_markdown(markdown)

    if title and (not markdown.startswith(f"# {title}\n") and markdown.strip() != f"# {title}"):
        markdown = f"# {title}\n\n{markdown}".strip()
    return markdown, {"title": title or None, "language": str(language or "") or None}


def _clean_html(html: str, base_url: str) -> BeautifulSoup:
    soup = BeautifulSoup(html or "", "html.parser")
    for node in soup(["script", "style", "noscript", "svg", "canvas", "iframe"]):
        node.decompose()

    main = soup.find("main") or soup.find("article") or soup.body or soup
    if main is not soup:
        for node in main.find_all(["nav", "header", "footer"]):
            node.decompose()

    for tag in soup.find_all(["a", "img"]):
        attr = "href" if tag.name == "a" else "src"
        value = str(tag.get(attr) or "").strip()
        if value and not value.startswith(("mailto:", "tel:", "javascript:", "#")):
            tag[attr] = urljoin(base_url, value)

    return soup


def _post_process_markdown(markdown: str) -> str:
    markdown = _process_multiline_links(markdown)
    markdown = re.sub(r"\[Skip to Content\]\(#[^)]+\)", "", markdown, flags=re.IGNORECASE)
    markdown = re.sub(r"\[Skip to content\]\(#[^)]+\)", "", markdown, flags=re.IGNORECASE)

    lines = [line.rstrip() for line in markdown.splitlines()]
    processed: list[str] = []
    blank_count = 0
    in_fenced_code = False

    for line in lines:
        if line.lstrip().startswith("```"):
            in_fenced_code = not in_fenced_code

        if not in_fenced_code and not line.strip():
            blank_count += 1
            if blank_count <= 2:
                processed.append("")
            continue

        blank_count = 0
        processed.append(line)

    return "\n".join(processed).strip()


def _process_multiline_links(markdown: str) -> str:
    inside_link_content = False
    link_open_count = 0
    output: list[str] = []

    for char in markdown:
        if char == "[":
            link_open_count += 1
        elif char == "]":
            link_open_count = max(0, link_open_count - 1)
        inside_link_content = link_open_count > 0

        if inside_link_content and char == "\n":
            output.append("\\\n")
        else:
            output.append(char)

    return "".join(output)


def fetch_page(url: str, *, timeout_seconds: int) -> ProviderPage:
    normalized = normalize_url(url)
    try:
        with httpx.Client(
            follow_redirects=True,
            timeout=timeout_seconds,
            headers={"User-Agent": USER_AGENT, "Accept": "text/html,application/xhtml+xml"},
        ) as client:
            response = client.get(normalized)
            response.raise_for_status()
    except httpx.HTTPStatusError as exc:
        status = exc.response.status_code if exc.response is not None else "unknown"
        raise fetch_failed(f"HTTP fetch failed with status {status}") from exc
    except httpx.HTTPError as exc:
        raise fetch_failed(str(exc)) from exc

    html = response.text or ""
    _, metadata = extract_markdown(html, str(response.url))
    return ProviderPage(
        url=normalized,
        final_url=str(response.url),
        html=html,
        status_code=response.status_code,
        title=metadata.get("title"),
        language=metadata.get("language"),
        provider="http_static",
        rendered=False,
    )


def discover_links(
    url: str,
    *,
    search: str | None,
    limit: int,
    include_subdomains: bool,
    ignore_sitemap: bool,
) -> tuple[list[str], str]:
    normalized = normalize_url(url)
    links: list[str] = []
    provider = "html_links"
    if not ignore_sitemap:
        links = _discover_sitemap_links(normalized, limit=limit)
        if links:
            provider = "sitemap"
    if not links:
        links = _discover_html_links(normalized, limit=limit)
    filtered = _filter_links(normalized, links, search=search, include_subdomains=include_subdomains)
    return (filtered or [normalized])[:limit], provider


def _discover_sitemap_links(url: str, *, limit: int) -> list[str]:
    parsed = urlparse(url)
    sitemap_url = f"{parsed.scheme}://{parsed.netloc}/sitemap.xml"
    try:
        with httpx.Client(follow_redirects=True, timeout=10, headers={"User-Agent": USER_AGENT}) as client:
            response = client.get(sitemap_url)
            response.raise_for_status()
    except httpx.HTTPError:
        return []
    try:
        root = ElementTree.fromstring(response.text.encode("utf-8"))
    except ElementTree.ParseError:
        return []
    links: list[str] = []
    for elem in root.iter():
        if elem.tag.endswith("loc") and elem.text:
            links.append(elem.text.strip())
            if len(links) >= limit:
                break
    return links


def _discover_html_links(url: str, *, limit: int) -> list[str]:
    try:
        page = fetch_page(url, timeout_seconds=15)
    except Exception:
        return [url]
    soup = BeautifulSoup(page.html or "", "html.parser")
    links = [url]
    for anchor in soup.find_all("a", href=True):
        href = str(anchor.get("href") or "").strip()
        if not href or href.startswith(("#", "mailto:", "tel:", "javascript:")):
            continue
        absolute = urldefrag(urljoin(page.final_url, href))[0]
        if absolute not in links:
            links.append(absolute)
        if len(links) >= limit:
            break
    return links


def _filter_links(base_url: str, links: list[str], *, search: str | None, include_subdomains: bool) -> list[str]:
    base_host = urlparse(base_url).hostname or ""
    needle = (search or "").strip().lower()
    filtered: list[str] = []
    for link in links:
        parsed = urlparse(link)
        host = parsed.hostname or ""
        same_site = host == base_host or (include_subdomains and host.endswith(f".{base_host}"))
        if not same_site:
            continue
        if needle and needle not in link.lower():
            continue
        if link not in filtered:
            filtered.append(link)
    return filtered


def _text(value: object) -> str:
    return re.sub(r"\s+", " ", str(value or "")).strip()
