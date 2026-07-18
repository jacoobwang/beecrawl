use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::time::Instant;

use ego_tree::NodeRef;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, USER_AGENT};
use scraper::{ElementRef, Html, Node, Selector};
use serde_json::json;
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::cache::CacheStore;
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
    scrape_with_cache(client, &CacheStore::from_env(), request).await
}

pub async fn scrape_with_cache(
    client: &reqwest::Client,
    cache: &CacheStore,
    request: WebExtractScrapeRequest,
) -> Result<WebExtractScrapeResponse, WebExtractError> {
    let started = Instant::now();
    let normalized = normalize_url(&request.url)?;
    let key = cache_key(&normalized, &request);
    let max_age_seconds = scrape_cache_max_age_seconds();
    let requires_screenshot = request
        .formats
        .iter()
        .any(|format| format.eq_ignore_ascii_case("screenshot"));
    let (mut page, _initial_markdown, _initial_meta, from_cache) =
        if let Some(page) = cache.get(&key, max_age_seconds, requires_screenshot).await {
            let (page, markdown, metadata) = page_to_markdown(page);
            (page, markdown, metadata, true)
        } else {
            let (page, markdown, metadata) = scrape_page(client, &request).await?;
            (page, markdown, metadata, false)
        };
    let configured_html = configured_content_html(&page.html, request.content.as_ref())?;
    let (markdown, markdown_meta) =
        extract_configured_markdown(&page.html, &configured_html, &page.final_url);
    if markdown.trim().is_empty() {
        return Err(WebExtractError::EmptyContent);
    }
    page.title = markdown_meta.get("title").cloned().flatten().or(page.title);
    page.language = markdown_meta
        .get("language")
        .cloned()
        .flatten()
        .or(page.language);
    if !from_cache {
        cache.put(&key, &page).await;
    }
    let wants_screenshot = request
        .formats
        .iter()
        .any(|format| format.eq_ignore_ascii_case("screenshot"));
    if wants_screenshot && page.screenshot.is_none() {
        return Err(WebExtractError::RenderFailed(
            "screenshot requires browser rendering".to_string(),
        ));
    }
    let html = request
        .formats
        .iter()
        .any(|format| format.eq_ignore_ascii_case("html"))
        .then(|| configured_html.clone());
    let raw_html = request
        .formats
        .iter()
        .any(|format| {
            format.eq_ignore_ascii_case("rawHtml") || format.eq_ignore_ascii_case("raw_html")
        })
        .then(|| page.html.clone());
    let links = request
        .formats
        .iter()
        .any(|format| format.eq_ignore_ascii_case("links"))
        .then(|| extract_links(&configured_html, &page.final_url));
    let screenshot = page.screenshot.map(|image| {
        if image.starts_with("data:") {
            image
        } else {
            format!("data:image/png;base64,{image}")
        }
    });
    Ok(WebExtractScrapeResponse {
        request_id: request_id("webext"),
        url: page.url,
        final_url: page.final_url,
        markdown,
        html,
        raw_html,
        links,
        screenshot,
        metadata: WebExtractMetadata {
            title: page.title,
            language: page.language,
            status_code: page.status_code,
            provider: page.provider,
            rendered: page.rendered,
            elapsed_ms: Some(started.elapsed().as_millis()),
        },
    })
}

fn cache_key(normalized_url: &str, request: &WebExtractScrapeRequest) -> String {
    let payload = serde_json::to_vec(&json!({
        "url": normalized_url,
        "use_browser": &request.use_browser,
        "wait_for_ms": request.wait_for_ms,
        "location": &request.location,
        "skip_tls_verification": request.skip_tls_verification,
        "headers": &request.headers,
        "proxy": &request.proxy,
        "screenshot": &request.screenshot,
    }))
    .expect("cache key payload serializes");
    format!("scrape:{:x}", Sha256::digest(payload))
}

fn scrape_cache_max_age_seconds() -> u64 {
    std::env::var("BEECRAWL_SCRAPE_CACHE_MAX_AGE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(4 * 60 * 60)
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
    let (links, provider) = discover_links(client, &normalized, &request, sitemap_mode).await?;
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
            let page = fetch_page_with_tls(
                client,
                &request.url,
                request.timeout_seconds,
                request.skip_tls_verification,
                &request.headers,
                request.proxy.as_ref(),
            )
            .await?;
            Ok(page_to_markdown(page))
        }
        _ => match render_page(client, request).await {
            Ok(page) => {
                let (_, markdown, metadata) = page_to_markdown(page.clone());
                if !markdown.trim().is_empty() {
                    Ok((page, markdown, metadata))
                } else {
                    let page = fetch_page_with_tls(
                        client,
                        &request.url,
                        request.timeout_seconds,
                        request.skip_tls_verification,
                        &request.headers,
                        request.proxy.as_ref(),
                    )
                    .await?;
                    Ok(page_to_markdown(page))
                }
            }
            Err(_) => {
                let page = fetch_page_with_tls(
                    client,
                    &request.url,
                    request.timeout_seconds,
                    request.skip_tls_verification,
                    &request.headers,
                    request.proxy.as_ref(),
                )
                .await?;
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
    fetch_page_with_tls(
        client,
        raw_url,
        timeout_seconds,
        false,
        &HashMap::new(),
        None,
    )
    .await
}

pub async fn robots_allows(
    client: &reqwest::Client,
    raw_url: &str,
    user_agent: Option<&str>,
    timeout_seconds: u64,
) -> bool {
    let Ok(url) = Url::parse(raw_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let robots_url = format!(
        "{}://{}{}/robots.txt",
        url.scheme(),
        host,
        url.port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default()
    );
    let agent = user_agent.unwrap_or("FirecrawlAgent");
    let response = client
        .get(robots_url)
        .header(USER_AGENT, agent)
        .timeout(std::time::Duration::from_secs(timeout_seconds.max(1)))
        .send()
        .await;
    let Ok(response) = response else {
        return true;
    };
    if !response.status().is_success() {
        return true;
    }
    let Ok(body) = response.text().await else {
        return true;
    };
    let allowed = robots_text_allows(&body, &url, agent);
    allowed
        && (url.path() == "/" || url.path().ends_with('/') || {
            let mut trailing = url.clone();
            trailing.set_path(&format!("{}/", url.path()));
            robots_text_allows(&body, &trailing, agent)
        })
}

#[derive(Clone)]
struct RobotsRule {
    allow: bool,
    pattern: String,
}

fn robots_text_allows(body: &str, url: &Url, user_agent: &str) -> bool {
    let requested_agent = user_agent.to_ascii_lowercase();
    let mut groups: Vec<(Vec<String>, Vec<RobotsRule>)> = Vec::new();
    let mut agents = Vec::new();
    let mut rules = Vec::new();
    let mut seen_rule = false;
    for raw_line in body.lines() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        let field = field.trim().to_ascii_lowercase();
        let value = value.trim();
        if field == "user-agent" {
            if seen_rule && !agents.is_empty() {
                groups.push((std::mem::take(&mut agents), std::mem::take(&mut rules)));
                seen_rule = false;
            }
            agents.push(value.to_ascii_lowercase());
        } else if matches!(field.as_str(), "allow" | "disallow") && !agents.is_empty() {
            seen_rule = true;
            if !value.is_empty() {
                rules.push(RobotsRule {
                    allow: field == "allow",
                    pattern: value.to_string(),
                });
            }
        }
    }
    if !agents.is_empty() {
        groups.push((agents, rules));
    }

    let best_agent_length = groups
        .iter()
        .flat_map(|(agents, _)| agents)
        .filter_map(|agent| robots_agent_match(agent, &requested_agent))
        .max();
    let Some(best_agent_length) = best_agent_length else {
        return true;
    };
    let mut path = url.path().to_string();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    groups
        .iter()
        .filter(|(agents, _)| {
            agents
                .iter()
                .any(|agent| robots_agent_match(agent, &requested_agent) == Some(best_agent_length))
        })
        .flat_map(|(_, rules)| rules)
        .filter_map(|rule| robots_pattern_match(&rule.pattern, &path).map(|length| (rule, length)))
        .max_by_key(|(rule, length)| (*length, rule.allow))
        .is_none_or(|(rule, _)| rule.allow)
}

fn robots_agent_match(rule_agent: &str, requested_agent: &str) -> Option<usize> {
    if rule_agent == "*" {
        Some(0)
    } else if requested_agent.contains(rule_agent) {
        Some(rule_agent.len())
    } else {
        None
    }
}

fn robots_pattern_match(pattern: &str, path: &str) -> Option<usize> {
    let anchored = pattern.ends_with('$');
    let pattern = pattern.strip_suffix('$').unwrap_or(pattern);
    let pieces = pattern.split('*').collect::<Vec<_>>();
    let mut offset = 0;
    for (index, piece) in pieces.iter().enumerate() {
        if piece.is_empty() {
            continue;
        }
        if index == 0 {
            if !path[offset..].starts_with(piece) {
                return None;
            }
            offset += piece.len();
        } else {
            let found = path[offset..].find(piece)?;
            offset += found + piece.len();
        }
    }
    if anchored && offset != path.len() {
        return None;
    }
    Some(
        pattern
            .chars()
            .filter(|character| *character != '*')
            .count(),
    )
}

async fn fetch_page_with_tls(
    client: &reqwest::Client,
    raw_url: &str,
    timeout_seconds: u64,
    skip_tls_verification: bool,
    headers: &HashMap<String, String>,
    proxy: Option<&crate::models::ProxyConfig>,
) -> Result<ProviderPage, WebExtractError> {
    let normalized = normalize_url(raw_url)?;
    let configured_client;
    let client = if skip_tls_verification || proxy.is_some() {
        let mut builder =
            reqwest::Client::builder().danger_accept_invalid_certs(skip_tls_verification);
        if let Some(proxy) = proxy {
            builder = builder.proxy(reqwest_proxy(proxy)?);
        }
        configured_client = builder
            .build()
            .map_err(|err| WebExtractError::FetchFailed(err.to_string()))?;
        &configured_client
    } else {
        client
    };
    let response = client
        .get(&normalized)
        .header(USER_AGENT, DEFAULT_USER_AGENT)
        .header(ACCEPT, "text/html,application/xhtml+xml")
        .headers(request_headers(headers)?)
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
        screenshot: None,
    })
}

async fn render_page(
    client: &reqwest::Client,
    request: &WebExtractScrapeRequest,
) -> Result<ProviderPage, WebExtractError> {
    let engine_url =
        std::env::var("BEE_ENGINE_URL").unwrap_or_else(|_| "http://127.0.0.1:8020".to_string());
    let screenshot = request.screenshot.clone().or_else(|| {
        request
            .formats
            .iter()
            .any(|format| format.eq_ignore_ascii_case("screenshot"))
            .then_some(crate::models::ScreenshotOptions {
                full_page: true,
                quality: None,
                viewport: None,
            })
    });
    let response = client
        .post(format!("{}/scrape", engine_url.trim_end_matches('/')))
        .json(&json!({
            "url": request.url,
            "engine": "playwright",
            "instantReturn": false,
            "timeout": request.timeout_seconds * 1000,
            "wait": request.wait_for_ms,
            "blockMedia": true,
            "skipTlsVerification": request.skip_tls_verification,
            "headers": request.headers,
            "proxy": request.proxy,
            "geolocation": request.location,
            "actions": if let Some(options) = &screenshot {
                json!([{
                    "type": "screenshot",
                    "fullPage": options.full_page,
                    "quality": options.quality,
                    "viewport": options.viewport,
                }])
            } else {
                json!([])
            },
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
        screenshot: rendered.screenshots.into_iter().next(),
    })
}

fn reqwest_proxy(config: &crate::models::ProxyConfig) -> Result<reqwest::Proxy, WebExtractError> {
    let mut proxy = reqwest::Proxy::all(&config.server)
        .map_err(|error| WebExtractError::FetchFailed(error.to_string()))?;
    if let Some(username) = &config.username {
        proxy = proxy.basic_auth(username, config.password.as_deref().unwrap_or_default());
    }
    Ok(proxy)
}

fn request_headers(headers: &HashMap<String, String>) -> Result<HeaderMap, WebExtractError> {
    let mut result = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
            WebExtractError::InvalidUrl(format!("invalid request header name: {error}"))
        })?;
        let value = HeaderValue::from_str(value).map_err(|error| {
            WebExtractError::InvalidUrl(format!("invalid request header value: {error}"))
        })?;
        result.insert(name, value);
    }
    Ok(result)
}

pub fn extract_markdown(html: &str, base_url: &str) -> (String, HashMap<String, Option<String>>) {
    let document = Html::parse_document(html);
    let title = select_first_text(&document, "title");
    let language = select_attr(&document, "html", "lang");
    let root_html = select_root_html(&document).unwrap_or_else(|| html.to_string());
    let root_doc = Html::parse_fragment(&root_html);
    let mut markdown = render_markdown(root_doc.root_element(), base_url);
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

pub fn extract_content_html(html: &str) -> String {
    let document = Html::parse_document(html);
    select_root_html(&document).unwrap_or_else(|| html.to_string())
}

fn configured_content_html(
    html: &str,
    options: Option<&crate::models::ContentOptions>,
) -> Result<String, WebExtractError> {
    let Some(options) = options else {
        return Ok(extract_content_html(html));
    };
    let document = Html::parse_document(html);
    let mut selected = if !options.include_tags.is_empty() {
        let mut fragments = Vec::new();
        for raw_selector in &options.include_tags {
            let selector = Selector::parse(raw_selector).map_err(|_| {
                WebExtractError::InvalidUrl(format!("invalid includeTags selector: {raw_selector}"))
            })?;
            fragments.extend(document.select(&selector).map(|node| node.html()));
        }
        fragments.join("\n")
    } else if options.only_main_content {
        select_root_html(&document).unwrap_or_else(|| html.to_string())
    } else {
        let body = Selector::parse("body").expect("valid body selector");
        document
            .select(&body)
            .next()
            .map(|node| node.html())
            .unwrap_or_else(|| document.html())
    };

    let mut exclusions = options.exclude_tags.clone();
    if options.only_clean_content {
        exclusions.extend(
            ["nav", "header", "footer", "aside", "form", "dialog"]
                .into_iter()
                .map(str::to_string),
        );
    }
    for raw_selector in exclusions {
        let selector = Selector::parse(&raw_selector).map_err(|_| {
            WebExtractError::InvalidUrl(format!("invalid excludeTags selector: {raw_selector}"))
        })?;
        let fragment = Html::parse_fragment(&selected);
        let removals = fragment
            .select(&selector)
            .map(|node| node.html())
            .collect::<Vec<_>>();
        for removal in removals {
            selected = selected.replace(&removal, "");
        }
    }
    Ok(selected)
}

fn extract_configured_markdown(
    original_html: &str,
    configured_html: &str,
    base_url: &str,
) -> (String, HashMap<String, Option<String>>) {
    let (_, metadata) = extract_markdown(original_html, base_url);
    let (mut markdown, _) = extract_markdown(configured_html, base_url);
    if let Some(title) = metadata.get("title").cloned().flatten() {
        if !title.is_empty() && !markdown.trim_start().starts_with(&format!("# {title}")) {
            markdown = format!("# {title}\n\n{}", markdown.trim());
        }
    }
    (collapse_blank_lines(&markdown), metadata)
}

pub fn extract_links(html: &str, base_url: &str) -> Vec<String> {
    let document = Html::parse_document(html);
    let root_html = select_root_html(&document).unwrap_or_else(|| html.to_string());
    let root_doc = Html::parse_fragment(&root_html);
    let selector = Selector::parse("a").expect("valid anchor selector");
    let base = Url::parse(base_url).ok();
    let mut seen = HashSet::new();
    let mut links = Vec::new();
    for node in root_doc.select(&selector) {
        let Some(href) = node.value().attr("href") else {
            continue;
        };
        if href.starts_with('#') || href.starts_with("javascript:") {
            continue;
        }
        let Some(url) = base.as_ref().and_then(|base| base.join(href).ok()) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        let url = url.to_string();
        if seen.insert(url.clone()) {
            links.push(url);
        }
    }
    links
}

pub fn extract_images(html: &str, base_url: &str) -> Vec<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("img").expect("valid image selector");
    let base = Url::parse(base_url).ok();
    let mut seen = HashSet::new();
    document
        .select(&selector)
        .filter_map(|node| node.value().attr("src"))
        .filter(|src| !src.starts_with("data:"))
        .filter_map(|src| base.as_ref().and_then(|base| base.join(src).ok()))
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| url.to_string())
        .filter(|url| seen.insert(url.clone()))
        .collect()
}

fn page_to_markdown(page: ProviderPage) -> (ProviderPage, String, HashMap<String, Option<String>>) {
    let (markdown, metadata) = extract_markdown(&page.html, &page.final_url);
    (page, markdown, metadata)
}

async fn discover_links(
    client: &reqwest::Client,
    normalized: &str,
    request: &WebExtractMapRequest,
    sitemap: &str,
) -> Result<(Vec<String>, String), WebExtractError> {
    let mut links = Vec::new();
    let mut providers = Vec::new();
    if matches!(sitemap, "include" | "only") {
        let sitemap_links = discover_sitemap_links(client, normalized, request.limit).await;
        if !sitemap_links.is_empty() {
            links.extend(sitemap_links);
            providers.push("sitemap");
        }
    }
    if sitemap != "only" {
        let html_links = discover_html_links(client, normalized, request.limit).await;
        if !html_links.is_empty() {
            links.extend(html_links);
            providers.push("html_links");
        }
    }
    let filtered = filter_links(
        normalized,
        links,
        request.search.as_deref(),
        request.include_subdomains,
        request.allow_external_links,
        request.crawl_entire_domain,
        request.ignore_query_parameters,
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
            .take(request.limit)
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
    allow_external_links: bool,
    crawl_entire_domain: bool,
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
        if !allow_external_links && !same_site {
            continue;
        }
        if !crawl_entire_domain && same_site && !parsed.path().starts_with(base.path()) {
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
    let keep_hash_route = parsed
        .fragment()
        .is_some_and(|fragment| fragment.starts_with('/') || fragment.starts_with("!/"));
    if !keep_hash_route {
        parsed.set_fragment(None);
    }
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

fn render_markdown(root: ElementRef<'_>, base_url: &str) -> String {
    let mut out = String::new();
    for child in root.children() {
        render_markdown_node(child, base_url, &mut out, false);
    }
    collapse_blank_lines(&out)
}

fn render_markdown_node(
    node: NodeRef<'_, Node>,
    base_url: &str,
    out: &mut String,
    preserve_whitespace: bool,
) {
    if let Some(text) = node.value().as_text() {
        if preserve_whitespace {
            out.push_str(text);
        } else {
            push_inline_text(out, text);
        }
        return;
    }
    let Some(element) = ElementRef::wrap(node) else {
        for child in node.children() {
            render_markdown_node(child, base_url, out, preserve_whitespace);
        }
        return;
    };
    let name = element.value().name();
    match name {
        "script" | "style" | "noscript" | "template" | "svg" => {}
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = name[1..].parse::<usize>().unwrap_or(1);
            let content = render_children(element, base_url, false);
            push_block(out, &format!("{} {}", "#".repeat(level), content.trim()));
        }
        "p" | "div" | "section" | "article" | "main" | "header" | "footer" | "aside" => {
            let content = render_children(element, base_url, false);
            push_block(out, content.trim());
        }
        "br" => out.push('\n'),
        "hr" => push_block(out, "---"),
        "strong" | "b" => {
            let content = render_children(element, base_url, false);
            if !content.trim().is_empty() {
                out.push_str("**");
                out.push_str(content.trim());
                out.push_str("**");
            }
        }
        "em" | "i" => {
            let content = render_children(element, base_url, false);
            if !content.trim().is_empty() {
                out.push('*');
                out.push_str(content.trim());
                out.push('*');
            }
        }
        "code"
            if node
                .parent()
                .and_then(ElementRef::wrap)
                .is_some_and(|p| p.value().name() == "pre") =>
        {
            for child in node.children() {
                render_markdown_node(child, base_url, out, true);
            }
        }
        "code" => {
            let content = render_children(element, base_url, true);
            if !content.is_empty() {
                out.push('`');
                out.push_str(content.trim());
                out.push('`');
            }
        }
        "pre" => {
            let content = element.text().collect::<String>();
            if !content.trim().is_empty() {
                push_block(out, &format!("```\n{}\n```", content.trim_matches('\n')));
            }
        }
        "a" => render_link(element, base_url, out),
        "img" => {
            let alt = element.attr("alt").unwrap_or("").trim();
            if let Some(src) = resolve_link(base_url, element.attr("src").unwrap_or("")) {
                out.push_str(&format!("![{alt}]({src})"));
            }
        }
        "ul" => render_list(element, base_url, out, false),
        "ol" => render_list(element, base_url, out, true),
        "li" => {
            let content = render_children(element, base_url, false);
            push_block(out, &format!("- {}", content.trim()));
        }
        "blockquote" => {
            let content = render_children(element, base_url, false);
            let quoted = content
                .trim()
                .lines()
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            push_block(out, &quoted);
        }
        "table" => render_table(element, out),
        _ => {
            for child in node.children() {
                render_markdown_node(child, base_url, out, preserve_whitespace);
            }
        }
    }
}

fn render_children(element: ElementRef<'_>, base_url: &str, preserve_whitespace: bool) -> String {
    let mut out = String::new();
    for child in element.children() {
        render_markdown_node(child, base_url, &mut out, preserve_whitespace);
    }
    out
}

fn render_link(element: ElementRef<'_>, base_url: &str, out: &mut String) {
    let content = render_children(element, base_url, false);
    let label = content.trim();
    let Some(href) = element
        .attr("href")
        .and_then(|href| resolve_link(base_url, href))
    else {
        out.push_str(label);
        return;
    };
    if label.is_empty() {
        out.push_str(&href);
    } else {
        out.push_str(&format!("[{label}]({href})"));
    }
}

fn resolve_link(base_url: &str, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('#') || raw.starts_with("javascript:") {
        return None;
    }
    let base = Url::parse(base_url).ok()?;
    let url = base.join(raw).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

fn render_list(element: ElementRef<'_>, base_url: &str, out: &mut String, ordered: bool) {
    let mut lines = Vec::new();
    for (index, item) in element
        .child_elements()
        .filter(|child| child.value().name() == "li")
        .enumerate()
    {
        let content = render_children(item, base_url, false);
        let prefix = if ordered {
            format!("{}. ", index + 1)
        } else {
            "- ".to_string()
        };
        let mut content_lines = content.trim().lines();
        if let Some(first) = content_lines.next() {
            lines.push(format!("{prefix}{first}"));
        }
        for line in content_lines {
            if !line.trim().is_empty() {
                lines.push(format!("  {line}"));
            }
        }
    }
    push_block(out, &lines.join("\n"));
}

fn render_table(element: ElementRef<'_>, out: &mut String) {
    let row_selector = Selector::parse("tr").expect("valid row selector");
    let mut rows = Vec::new();
    for row in element.select(&row_selector) {
        let cells = row
            .child_elements()
            .filter(|cell| matches!(cell.value().name(), "th" | "td"))
            .map(|cell| collect_text(&cell.text().collect::<Vec<_>>().join(" ")))
            .collect::<Vec<_>>();
        if !cells.is_empty() {
            rows.push(cells);
        }
    }
    let Some(width) = rows.first().map(Vec::len) else {
        return;
    };
    let mut lines = Vec::new();
    for (index, mut row) in rows.into_iter().enumerate() {
        row.resize(width, String::new());
        lines.push(format!("| {} |", row.join(" | ")));
        if index == 0 {
            lines.push(format!("| {} |", vec!["---"; width].join(" | ")));
        }
    }
    push_block(out, &lines.join("\n"));
}

fn push_inline_text(out: &mut String, text: &str) {
    let has_leading_whitespace = text.chars().next().is_some_and(char::is_whitespace);
    let has_trailing_whitespace = text.chars().next_back().is_some_and(char::is_whitespace);
    let normalized = collect_text(text);
    if normalized.is_empty() {
        return;
    }
    if has_leading_whitespace && out.chars().last().is_some_and(|last| !last.is_whitespace()) {
        out.push(' ');
    }
    out.push_str(&normalized);
    if has_trailing_whitespace && !out.ends_with(char::is_whitespace) {
        out.push(' ');
    }
}

fn push_block(out: &mut String, content: &str) {
    let content = content.trim();
    if content.is_empty() {
        return;
    }
    if !out.trim_end().is_empty() {
        while out.ends_with(char::is_whitespace) {
            out.pop();
        }
        out.push_str("\n\n");
    }
    out.push_str(content);
    out.push_str("\n\n");
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
    fn markdown_preserves_document_order_and_block_structure() {
        let html = include_str!("../tests/fixtures/article.html");
        let (markdown, _) = extract_markdown(html, "https://example.com/articles/ordered");

        let heading = markdown.find("# First heading").unwrap();
        let paragraph = markdown.find("Opening paragraph").unwrap();
        let second_heading = markdown.find("## Second heading").unwrap();
        let quote = markdown.find("> A quoted conclusion.").unwrap();
        let code = markdown.find("```\nconst answer = 42;").unwrap();
        assert!(heading < paragraph);
        assert!(paragraph < second_heading);
        assert!(second_heading < quote);
        assert!(quote < code);
        assert!(markdown.contains(
            "Opening paragraph with **important text** and [a guide](https://example.com/guide)."
        ));
        assert!(markdown.contains("**important text**"));
        assert!(markdown.contains("[a guide](https://example.com/guide)"));
    }

    #[test]
    fn markdown_renders_lists_tables_and_images() {
        let html = include_str!("../tests/fixtures/catalog.html");
        let (markdown, _) = extract_markdown(html, "https://example.com/catalog");

        assert!(markdown.contains("- Thermal pad\n- Thermal paste"));
        assert!(markdown.contains("| Product | Conductivity |"));
        assert!(markdown.contains("| --- | --- |"));
        assert!(markdown.contains("| TP-10 | 10 W/mK |"));
        assert!(markdown.contains("![TP-10 package](https://example.com/images/tp-10.png)"));
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
    fn content_html_uses_the_selected_content_root() {
        let html = r#"<html><body><header>Navigation</header><main><h1>Article</h1><p>Content</p></main><footer>Footer</footer></body></html>"#;
        let content_html = extract_content_html(html);

        assert!(content_html.contains("<main>"));
        assert!(content_html.contains("Article"));
        assert!(!content_html.contains("Navigation"));
    }

    #[test]
    fn links_are_absolute_and_deduplicated() {
        let html = r##"<html><body><main><a href="/docs">Docs</a><a href="https://example.com/docs">Docs again</a><a href="#part">Part</a></main></body></html>"##;
        let links = extract_links(html, "https://example.com/start");

        assert_eq!(links, vec!["https://example.com/docs".to_string()]);
    }

    #[test]
    fn link_canonicalization_respects_query_and_hash_route_policy() {
        assert_eq!(
            canonicalize_link("https://example.com/a?x=1#section", false).unwrap(),
            "https://example.com/a?x=1"
        );
        assert_eq!(
            canonicalize_link("https://example.com/a?x=1#/route", true).unwrap(),
            "https://example.com/a#/route"
        );
    }

    #[test]
    fn map_domain_policies_expand_scope_explicitly() {
        let links = vec![
            "https://example.com/docs/page".to_string(),
            "https://example.com/other".to_string(),
            "https://api.example.com/docs/api".to_string(),
            "https://external.test/page".to_string(),
        ];
        assert_eq!(
            filter_links(
                "https://example.com/docs",
                links.clone(),
                None,
                false,
                false,
                false,
                false,
            ),
            ["https://example.com/docs/page"]
        );
        assert_eq!(
            filter_links(
                "https://example.com/docs",
                links.clone(),
                None,
                true,
                false,
                true,
                false,
            )
            .len(),
            3
        );
        assert_eq!(
            filter_links(
                "https://example.com/docs",
                links,
                None,
                false,
                true,
                false,
                false,
            )
            .len(),
            3
        );
    }

    #[test]
    fn images_are_absolute_deduplicated_and_exclude_data_urls() {
        let html = r#"<html><body>
            <img src="/hero.png"><img src="/hero.png">
            <img src="https://cdn.example.com/photo.jpg">
            <img src="data:image/png;base64,AAAA">
        </body></html>"#;
        assert_eq!(
            extract_images(html, "https://example.com/docs/page"),
            [
                "https://example.com/hero.png",
                "https://cdn.example.com/photo.jpg"
            ]
        );
    }

    #[test]
    fn custom_request_headers_are_validated() {
        let headers = HashMap::from([
            ("Authorization".to_string(), "Bearer token".to_string()),
            ("X-Tenant".to_string(), "example".to_string()),
        ]);
        let parsed = request_headers(&headers).unwrap();
        assert_eq!(parsed.get("x-tenant").unwrap(), "example");
        assert!(request_headers(&HashMap::from([(
            "bad header".to_string(),
            "value".to_string()
        )]))
        .is_err());
    }

    #[test]
    fn content_options_select_and_clean_html() {
        let html = r#"<html><head><title>Example</title></head><body>
            <header>Navigation</header>
            <main><section class="keep"><p>Keep me</p><span class="remove">Remove me</span></section></main>
            <footer>Footer</footer>
        </body></html>"#;
        let selected = configured_content_html(
            html,
            Some(&crate::models::ContentOptions {
                only_main_content: false,
                only_clean_content: true,
                include_tags: vec![".keep".to_string()],
                exclude_tags: vec![".remove".to_string()],
            }),
        )
        .unwrap();
        assert!(selected.contains("Keep me"));
        assert!(!selected.contains("Remove me"));
        assert!(!selected.contains("Navigation"));
    }

    #[test]
    fn only_main_content_false_keeps_page_chrome() {
        let html =
            r#"<html><body><header>Navigation</header><main><p>Article</p></main></body></html>"#;
        let selected = configured_content_html(
            html,
            Some(&crate::models::ContentOptions {
                only_main_content: false,
                only_clean_content: false,
                include_tags: Vec::new(),
                exclude_tags: Vec::new(),
            }),
        )
        .unwrap();
        assert!(selected.contains("Navigation"));
        assert!(selected.contains("Article"));
    }

    #[test]
    fn normalize_blocks_localhost() {
        assert!(matches!(
            normalize_url("http://127.0.0.1:8000"),
            Err(WebExtractError::BlockedByPolicy(_))
        ));
    }

    #[test]
    fn robots_rules_use_the_most_specific_agent_and_path() {
        let robots = r#"
            User-agent: *
            Disallow: /private
            Allow: /private/public

            User-agent: BeeBot
            Disallow: /bee
            Allow: /bee/feed$
        "#;
        let url = |path: &str| Url::parse(&format!("https://example.com{path}")).unwrap();

        assert!(!robots_text_allows(
            robots,
            &url("/private/report"),
            "OtherBot"
        ));
        assert!(robots_text_allows(
            robots,
            &url("/private/public"),
            "OtherBot"
        ));
        assert!(!robots_text_allows(
            robots,
            &url("/bee/report"),
            "BeeBot/1.0"
        ));
        assert!(robots_text_allows(robots, &url("/bee/feed"), "BeeBot/1.0"));
        assert!(!robots_text_allows(
            robots,
            &url("/bee/feed/more"),
            "BeeBot/1.0"
        ));
    }

    #[test]
    fn robots_allow_wins_when_rules_have_equal_specificity() {
        let robots = "User-agent: *\nDisallow: /same\nAllow: /same";
        let url = Url::parse("https://example.com/same").unwrap();
        assert!(robots_text_allows(robots, &url, "FirecrawlAgent"));
    }
}
