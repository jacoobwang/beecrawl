use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::time::Instant;

use reqwest::header::{ACCEPT, USER_AGENT};
use scraper::{Html, Selector};
use serde_json::json;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::models::{
    BeeEngineScrapeResponse, ProviderPage, WebExtractMapMetadata, WebExtractMapRequest,
    WebExtractMapResponse, WebExtractMetadata, WebExtractScrapeRequest, WebExtractScrapeResponse,
};

const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
const MIN_CONTENT_ROOT_CHARS: usize = 500;

#[derive(Debug, Error)]
pub enum WebExtractError {
    #[error("invalid_url: {0}")]
    InvalidUrl(String),
    #[error("blocked_by_policy: {0}")]
    BlockedByPolicy(String),
    #[error("fetch_failed: {0}")]
    FetchFailed(String),
    #[error("render_failed: {0}")]
    RenderFailed(String),
    #[error("empty_content: No content could be extracted")]
    EmptyContent,
}

impl WebExtractError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUrl(_) => "invalid_url",
            Self::BlockedByPolicy(_) => "blocked_by_policy",
            Self::FetchFailed(_) => "fetch_failed",
            Self::RenderFailed(_) => "render_failed",
            Self::EmptyContent => "empty_content",
        }
    }

    pub fn status(&self) -> axum::http::StatusCode {
        match self {
            Self::InvalidUrl(_) => axum::http::StatusCode::BAD_REQUEST,
            Self::BlockedByPolicy(_) => axum::http::StatusCode::FORBIDDEN,
            Self::FetchFailed(_) | Self::RenderFailed(_) => axum::http::StatusCode::BAD_GATEWAY,
            Self::EmptyContent => axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        }
    }
}

pub async fn scrape(
    client: &reqwest::Client,
    request: WebExtractScrapeRequest,
) -> Result<WebExtractScrapeResponse, WebExtractError> {
    let started = Instant::now();
    let (page, markdown, markdown_meta) = scrape_page(client, &request).await?;
    if markdown.trim().is_empty() {
        return Err(WebExtractError::EmptyContent);
    }
    Ok(WebExtractScrapeResponse {
        request_id: request_id("webext"),
        url: page.url,
        final_url: page.final_url,
        markdown,
        metadata: WebExtractMetadata {
            title: markdown_meta.get("title").cloned().flatten().or(page.title),
            language: markdown_meta
                .get("language")
                .cloned()
                .flatten()
                .or(page.language),
            status_code: page.status_code,
            provider: page.provider,
            rendered: page.rendered,
            elapsed_ms: Some(started.elapsed().as_millis()),
        },
    })
}

pub async fn map_site(
    client: &reqwest::Client,
    request: WebExtractMapRequest,
) -> Result<WebExtractMapResponse, WebExtractError> {
    let started = Instant::now();
    let normalized = normalize_url(&request.url)?;
    let sitemap_mode = if request.ignore_sitemap {
        "skip"
    } else {
        request.sitemap.as_str()
    };
    let (links, provider) = discover_links(
        client,
        &normalized,
        request.search.as_deref(),
        request.limit,
        request.include_subdomains,
        sitemap_mode,
        request.ignore_query_parameters,
    )
    .await?;
    Ok(WebExtractMapResponse {
        request_id: request_id("webext"),
        url: normalized,
        metadata: WebExtractMapMetadata {
            provider,
            count: links.len(),
            elapsed_ms: Some(started.elapsed().as_millis()),
        },
        links,
    })
}

async fn scrape_page(
    client: &reqwest::Client,
    request: &WebExtractScrapeRequest,
) -> Result<(ProviderPage, String, HashMap<String, Option<String>>), WebExtractError> {
    match request.use_browser.as_str() {
        "always" => {
            let page = render_page(client, request).await?;
            Ok(page_to_markdown(page))
        }
        "never" => {
            let page = fetch_page(client, &request.url, request.timeout_seconds).await?;
            Ok(page_to_markdown(page))
        }
        _ => match render_page(client, request).await {
            Ok(page) => {
                let (_, markdown, metadata) = page_to_markdown(page.clone());
                if !markdown.trim().is_empty() {
                    Ok((page, markdown, metadata))
                } else {
                    let page = fetch_page(client, &request.url, request.timeout_seconds).await?;
                    Ok(page_to_markdown(page))
                }
            }
            Err(_) => {
                let page = fetch_page(client, &request.url, request.timeout_seconds).await?;
                Ok(page_to_markdown(page))
            }
        },
    }
}

pub async fn fetch_page(
    client: &reqwest::Client,
    raw_url: &str,
    timeout_seconds: u64,
) -> Result<ProviderPage, WebExtractError> {
    let normalized = normalize_url(raw_url)?;
    let response = client
        .get(&normalized)
        .header(USER_AGENT, DEFAULT_USER_AGENT)
        .header(ACCEPT, "text/html,application/xhtml+xml")
        .timeout(std::time::Duration::from_secs(timeout_seconds))
        .send()
        .await
        .map_err(|err| WebExtractError::FetchFailed(err.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(WebExtractError::FetchFailed(format!(
            "HTTP fetch failed with status {}",
            status.as_u16()
        )));
    }
    let final_url = response.url().to_string();
    let html = response
        .text()
        .await
        .map_err(|err| WebExtractError::FetchFailed(err.to_string()))?;
    let (_, metadata) = extract_markdown(&html, &final_url);
    Ok(ProviderPage {
        url: normalized,
        final_url,
        html,
        status_code: Some(status.as_u16()),
        title: metadata.get("title").cloned().flatten(),
        language: metadata.get("language").cloned().flatten(),
        provider: "http_static".to_string(),
        rendered: false,
    })
}

async fn render_page(
    client: &reqwest::Client,
    request: &WebExtractScrapeRequest,
) -> Result<ProviderPage, WebExtractError> {
    let engine_url =
        std::env::var("BEE_ENGINE_URL").unwrap_or_else(|_| "http://127.0.0.1:8020".to_string());
    let response = client
        .post(format!("{}/scrape", engine_url.trim_end_matches('/')))
        .json(&json!({
            "url": request.url,
            "engine": "playwright",
            "instantReturn": false,
            "timeout": request.timeout_seconds * 1000,
            "wait": request.wait_for_ms,
            "blockMedia": true,
            "geolocation": request.location,
        }))
        .timeout(std::time::Duration::from_secs(request.timeout_seconds + 5))
        .send()
        .await
        .map_err(|err| WebExtractError::RenderFailed(err.to_string()))?;
    if !response.status().is_success() {
        return Err(WebExtractError::RenderFailed(format!(
            "Bee Engine failed with status {}",
            response.status().as_u16()
        )));
    }
    let rendered: BeeEngineScrapeResponse = response
        .json()
        .await
        .map_err(|err| WebExtractError::RenderFailed(err.to_string()))?;
    if let Some(error) = rendered.page_error {
        if !error.trim().is_empty() {
            return Err(WebExtractError::RenderFailed(error));
        }
    }
    Ok(ProviderPage {
        url: request.url.clone(),
        final_url: rendered.url,
        html: rendered.content,
        status_code: rendered.page_status_code,
        title: None,
        language: None,
        provider: "bee_engine".to_string(),
        rendered: true,
    })
}

pub fn extract_markdown(html: &str, base_url: &str) -> (String, HashMap<String, Option<String>>) {
    let document = Html::parse_document(html);
    let title = select_first_text(&document, "title");
    let language = select_attr(&document, "html", "lang");
    let root_html = select_root_html(&document).unwrap_or_else(|| html.to_string());
    let root_doc = Html::parse_fragment(&root_html);
    let mut markdown = String::new();
    append_markdown_from_selector(&root_doc, "h1", "# ", &mut markdown);
    append_markdown_from_selector(&root_doc, "h2", "## ", &mut markdown);
    append_markdown_from_selector(&root_doc, "h3", "### ", &mut markdown);
    append_markdown_from_selector(&root_doc, "p", "", &mut markdown);
    append_markdown_from_selector(&root_doc, "li", "- ", &mut markdown);
    append_links(&root_doc, base_url, &mut markdown);
    if markdown.trim().is_empty() {
        markdown = collect_text(&root_doc.root_element().text().collect::<Vec<_>>().join(" "));
    }
    if let Some(title) = title.clone() {
        if !title.is_empty() && !markdown.trim_start().starts_with(&format!("# {title}")) {
            markdown = format!("# {title}\n\n{}", markdown.trim())
                .trim()
                .to_string();
        }
    }
    let mut metadata = HashMap::new();
    metadata.insert("title".to_string(), title.filter(|x| !x.is_empty()));
    metadata.insert("language".to_string(), language.filter(|x| !x.is_empty()));
    (collapse_blank_lines(&markdown), metadata)
}

fn page_to_markdown(page: ProviderPage) -> (ProviderPage, String, HashMap<String, Option<String>>) {
    let (markdown, metadata) = extract_markdown(&page.html, &page.final_url);
    (page, markdown, metadata)
}

async fn discover_links(
    client: &reqwest::Client,
    normalized: &str,
    search: Option<&str>,
    limit: usize,
    include_subdomains: bool,
    sitemap: &str,
    ignore_query_parameters: bool,
) -> Result<(Vec<String>, String), WebExtractError> {
    let mut links = Vec::new();
    let mut providers = Vec::new();
    if matches!(sitemap, "include" | "only") {
        let sitemap_links = discover_sitemap_links(client, normalized, limit).await;
        if !sitemap_links.is_empty() {
            links.extend(sitemap_links);
            providers.push("sitemap");
        }
    }
    if sitemap != "only" {
        let html_links = discover_html_links(client, normalized, limit).await;
        if !html_links.is_empty() {
            links.extend(html_links);
            providers.push("html_links");
        }
    }
    let filtered = filter_links(
        normalized,
        links,
        search,
        include_subdomains,
        ignore_query_parameters,
    );
    let provider = if providers.is_empty() {
        "html_links".to_string()
    } else {
        providers.join("+")
    };
    Ok((
        (if filtered.is_empty() {
            vec![normalized.to_string()]
        } else {
            filtered
        })[..]
            .iter()
            .take(limit)
            .cloned()
            .collect(),
        provider,
    ))
}

async fn discover_sitemap_links(client: &reqwest::Client, url: &str, limit: usize) -> Vec<String> {
    let Ok(parsed) = Url::parse(url) else {
        return vec![];
    };
    let Some(host) = parsed.host_str() else {
        return vec![];
    };
    let sitemap_url = format!("{}://{}/sitemap.xml", parsed.scheme(), host);
    let Ok(response) = client
        .get(sitemap_url)
        .header(USER_AGENT, DEFAULT_USER_AGENT)
        .send()
        .await
    else {
        return vec![];
    };
    let Ok(text) = response.text().await else {
        return vec![];
    };
    text.split("<loc>")
        .skip(1)
        .filter_map(|part| part.split("</loc>").next())
        .map(html_escape::decode_html_entities)
        .map(|url| url.to_string())
        .take(limit)
        .collect()
}

async fn discover_html_links(client: &reqwest::Client, url: &str, limit: usize) -> Vec<String> {
    let Ok(page) = fetch_page(client, url, 20).await else {
        return vec![];
    };
    let document = Html::parse_document(&page.html);
    let selector = Selector::parse("a").unwrap();
    document
        .select(&selector)
        .filter_map(|node| node.value().attr("href"))
        .filter_map(|href| Url::parse(&page.final_url).ok()?.join(href).ok())
        .map(|url| url.to_string())
        .take(limit)
        .collect()
}

fn filter_links(
    base_url: &str,
    links: Vec<String>,
    search: Option<&str>,
    include_subdomains: bool,
    ignore_query_parameters: bool,
) -> Vec<String> {
    let Ok(base) = Url::parse(base_url) else {
        return links;
    };
    let base_host = base.host_str().unwrap_or("").trim_start_matches("www.");
    let search = search.map(|x| x.to_lowercase());
    let mut seen = HashSet::new();
    let mut filtered = Vec::new();
    for link in links {
        let Some(canonical) = canonicalize_link(&link, ignore_query_parameters) else {
            continue;
        };
        let Ok(parsed) = Url::parse(&canonical) else {
            continue;
        };
        let host = parsed.host_str().unwrap_or("").trim_start_matches("www.");
        let same_site =
            host == base_host || (include_subdomains && host.ends_with(&format!(".{base_host}")));
        if !same_site {
            continue;
        }
        if let Some(search) = &search {
            if !canonical.to_lowercase().contains(search) {
                continue;
            }
        }
        if seen.insert(canonical.clone()) {
            filtered.push(canonical);
        }
    }
    filtered
}

fn canonicalize_link(link: &str, ignore_query_parameters: bool) -> Option<String> {
    let mut parsed = Url::parse(link).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    parsed.set_fragment(None);
    if ignore_query_parameters {
        parsed.set_query(None);
    }
    if parsed.path().is_empty() {
        parsed.set_path("/");
    }
    Some(parsed.to_string())
}

pub fn normalize_url(raw_url: &str) -> Result<String, WebExtractError> {
    let value = raw_url.trim();
    if value.is_empty() {
        return Err(WebExtractError::InvalidUrl("URL is required".to_string()));
    }
    let value = if value.starts_with("http://") || value.starts_with("https://") {
        value.to_string()
    } else {
        format!("https://{value}")
    };
    let parsed = Url::parse(&value).map_err(|_| {
        WebExtractError::InvalidUrl("Only http and https URLs are supported".to_string())
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(WebExtractError::InvalidUrl(
            "Only http and https URLs are supported".to_string(),
        ));
    }
    let host = parsed.host_str().unwrap_or("");
    if host == "localhost" || host.ends_with(".localhost") {
        return Err(WebExtractError::BlockedByPolicy(
            "Localhost URLs are not allowed".to_string(),
        ));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        let blocked = match ip {
            IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
            IpAddr::V6(ip) => {
                ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
            }
        };
        if blocked {
            return Err(WebExtractError::BlockedByPolicy(
                "Private network URLs are not allowed".to_string(),
            ));
        }
    }
    Ok(value)
}

fn select_root_html(document: &Html) -> Option<String> {
    let body_selector = Selector::parse("body").ok()?;
    let body = document.select(&body_selector).next();
    let body_text_len = body
        .as_ref()
        .map(|node| collect_text(&node.text().collect::<Vec<_>>().join(" ")).len())
        .unwrap_or(0);

    for selector in ["main", "article"] {
        let selector = Selector::parse(selector).ok()?;
        if let Some(node) = document.select(&selector).next() {
            let preferred_text_len = collect_text(&node.text().collect::<Vec<_>>().join(" ")).len();
            if preferred_text_len < MIN_CONTENT_ROOT_CHARS
                && body_text_len >= MIN_CONTENT_ROOT_CHARS.max(preferred_text_len * 3)
            {
                break;
            }
            return Some(node.html());
        }
    }
    body.map(|node| node.html())
        .or_else(|| Some(document.html()))
}

fn append_markdown_from_selector(document: &Html, selector: &str, prefix: &str, out: &mut String) {
    let selector = Selector::parse(selector).unwrap();
    for node in document.select(&selector) {
        let text = collect_text(&node.text().collect::<Vec<_>>().join(" "));
        if !text.is_empty() {
            out.push_str(prefix);
            out.push_str(&text);
            out.push_str("\n\n");
        }
    }
}

fn append_links(document: &Html, base_url: &str, out: &mut String) {
    let selector = Selector::parse("a").unwrap();
    let base = Url::parse(base_url).ok();
    for node in document.select(&selector) {
        let text = collect_text(&node.text().collect::<Vec<_>>().join(" "));
        if text.is_empty() {
            continue;
        }
        let Some(href) = node.value().attr("href") else {
            continue;
        };
        if href.starts_with('#') || href.starts_with("javascript:") {
            continue;
        }
        let url = base
            .as_ref()
            .and_then(|base| base.join(href).ok())
            .map(|url| url.to_string())
            .unwrap_or_else(|| href.to_string());
        out.push_str(&format!("[{text}]({url})\n\n"));
    }
}

fn select_first_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .map(|node| collect_text(&node.text().collect::<Vec<_>>().join(" ")))
}

fn select_attr(document: &Html, selector: &str, attr: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|node| node.value().attr(attr))
        .map(str::to_string)
}

fn collect_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collapse_blank_lines(markdown: &str) -> String {
    let mut out = Vec::new();
    let mut blanks = 0;
    for line in markdown.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks <= 1 {
                out.push("");
            }
        } else {
            blanks = 0;
            out.push(line);
        }
    }
    out.join("\n").trim().to_string()
}

fn request_id(prefix: &str) -> String {
    format!("{}_{}", prefix, Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_keeps_title_and_links() {
        let html = r#"<html lang="en"><head><title>Acme</title></head><body><main><h1>About</h1><p>Hello <a href="/x">Link</a></p></main></body></html>"#;
        let (markdown, metadata) = extract_markdown(html, "https://example.com");
        assert!(markdown.contains("# Acme"));
        assert!(markdown.contains("# About"));
        assert!(markdown.contains("[Link](https://example.com/x)"));
        assert_eq!(metadata["language"], Some("en".to_string()));
    }

    #[test]
    fn markdown_falls_back_to_body_when_main_is_sparse() {
        let body_content = "Rendered body content. ".repeat(40);
        let html = format!(
            r#"<html><head><title>Y Warm</title></head><body>
              <header>Navigation</header>
              <main><a href="/report"><h2>Report</h2><p>View More</p></a></main>
              <section><h1>Thermal Material</h1><p>{body_content}</p></section>
              <footer>Copyright</footer>
            </body></html>"#
        );
        let (markdown, _) = extract_markdown(&html, "https://example.com");

        assert!(markdown.contains("Thermal Material"));
        assert!(markdown.contains("Rendered body content."));
        assert!(markdown.contains("Report"));
    }

    #[test]
    fn normalize_blocks_localhost() {
        assert!(matches!(
            normalize_url("http://127.0.0.1:8000"),
            Err(WebExtractError::BlockedByPolicy(_))
        ));
    }
}
