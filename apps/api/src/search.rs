use std::collections::HashMap;
use std::time::Instant;

use futures::{stream, StreamExt};
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, USER_AGENT};
use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json::Value;
use url::Url;
use uuid::Uuid;

use crate::models::{
    SearchMetadata, SearchRequest, SearchResponse, SearchResult, WebExtractScrapeRequest,
};
use crate::web_extract;

const DEFAULT_USER_AGENT: &str = "BeeCrawl/0.1";

#[derive(Debug, Clone)]
struct ProviderResult {
    url: String,
    title: Option<String>,
    description: Option<String>,
}

#[derive(Debug)]
struct ProviderResponse {
    provider: String,
    results: Vec<ProviderResult>,
}

#[derive(Debug, Deserialize)]
struct SearxngResponse {
    #[serde(default)]
    results: Vec<SearxngResult>,
}

#[derive(Debug, Deserialize)]
struct SearxngResult {
    url: Option<String>,
    title: Option<String>,
    content: Option<String>,
}

pub async fn search(
    client: &reqwest::Client,
    request: SearchRequest,
) -> anyhow::Result<SearchResponse> {
    let started = Instant::now();
    let query = build_search_query(
        &request.query,
        &request.categories,
        &request.include_domains,
        &request.exclude_domains,
    );
    let provider = search_web(
        client,
        &query,
        request.limit,
        &request.lang,
        &request.country,
        request.tbs.as_deref(),
        request.location.as_deref(),
        request.filter.as_deref(),
    )
    .await;
    let mut results: Vec<SearchResult> = provider
        .results
        .into_iter()
        .take(request.limit)
        .map(|item| SearchResult {
            url: item.url,
            title: item.title,
            description: item.description,
            markdown: None,
            metadata: HashMap::new(),
            scrape_error: None,
            highlights: vec![],
        })
        .filter(|result| {
            domain_allowed(
                &result.url,
                &request.include_domains,
                &request.exclude_domains,
            )
        })
        .collect();

    for result in &mut results {
        if let Some(category) = category_for_url(&result.url, &request.categories) {
            result
                .metadata
                .insert("category".to_string(), Value::String(category));
        }
        if request.highlights {
            let source = result.description.as_deref().unwrap_or_default();
            result.highlights = query_highlights(source, &request.query);
        }
    }

    let should_scrape = request
        .scrape_options
        .as_ref()
        .map(|options| !options.formats.is_empty())
        .unwrap_or(false);
    if should_scrape {
        let options = request.scrape_options.clone().unwrap();
        if request.async_scraping {
            let options_ref = &options;
            let query_ref = &request.query;
            let highlights = request.highlights;
            let mut scraped = stream::iter(results.into_iter().enumerate())
                .map(|(index, result)| async move {
                    (
                        index,
                        scrape_result(client, result, options_ref, highlights, query_ref).await,
                    )
                })
                .buffer_unordered(8)
                .collect::<Vec<_>>()
                .await;
            scraped.sort_by_key(|(index, _)| *index);
            results = scraped.into_iter().map(|(_, result)| result).collect();
        } else {
            for result in &mut results {
                *result = scrape_result(
                    client,
                    result.clone(),
                    &options,
                    request.highlights,
                    &request.query,
                )
                .await;
            }
        }
    }

    let scraped_count = results
        .iter()
        .filter(|item| item.markdown.is_some())
        .count();
    Ok(SearchResponse {
        request_id: format!("search_{}", Uuid::new_v4().simple()),
        query: request.query,
        metadata: SearchMetadata {
            provider: provider.provider,
            count: results.len(),
            scraped_count,
            elapsed_ms: Some(started.elapsed().as_millis()),
        },
        results,
    })
}

async fn scrape_result(
    client: &reqwest::Client,
    mut result: SearchResult,
    options: &crate::models::SearchScrapeOptions,
    highlights: bool,
    query: &str,
) -> SearchResult {
    match web_extract::scrape(
        client,
        WebExtractScrapeRequest {
            url: result.url.clone(),
            formats: options.formats.clone(),
            location: None,
            timeout_seconds: options.timeout_seconds,
            wait_for_ms: options.wait_for_ms,
            use_browser: options.use_browser.clone(),
            skip_tls_verification: options.skip_tls_verification,
            headers: options.headers.clone(),
            proxy: options.proxy.clone(),
            screenshot: None,
            content: options.content.clone(),
            actions: options.actions.clone(),
        },
    )
    .await
    {
        Ok(scrape) => {
            if highlights {
                result.highlights = query_highlights(&scrape.markdown, query);
            }
            result.markdown = Some(scrape.markdown);
            result
                .metadata
                .insert("final_url".to_string(), Value::String(scrape.final_url));
            result.metadata.insert(
                "provider".to_string(),
                Value::String(scrape.metadata.provider),
            );
            result.metadata.insert(
                "rendered".to_string(),
                Value::Bool(scrape.metadata.rendered),
            );
        }
        Err(error) => result.scrape_error = Some(error.code().to_string()),
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn search_web(
    client: &reqwest::Client,
    query: &str,
    limit: usize,
    lang: &str,
    country: &str,
    tbs: Option<&str>,
    location: Option<&str>,
    filter: Option<&str>,
) -> ProviderResponse {
    if let Ok(endpoint) = std::env::var("BEECRAWL_SEARXNG_ENDPOINT") {
        if !endpoint.trim().is_empty() {
            let response =
                search_searxng(client, endpoint.trim(), query, limit, lang, tbs, filter).await;
            if !response.results.is_empty() {
                return response;
            }
        }
    }
    search_duckduckgo(client, query, limit, lang, country, tbs, location, filter).await
}

async fn search_searxng(
    client: &reqwest::Client,
    endpoint: &str,
    query: &str,
    limit: usize,
    lang: &str,
    tbs: Option<&str>,
    filter: Option<&str>,
) -> ProviderResponse {
    let url = format!("{}/search", endpoint.trim_end_matches('/'));
    let response = client
        .get(url)
        .query(&[
            ("q", query),
            ("language", lang),
            ("format", "json"),
            ("time_range", tbs.unwrap_or_default()),
            ("safesearch", filter.unwrap_or_default()),
        ])
        .send()
        .await;
    let Ok(response) = response else {
        return ProviderResponse {
            provider: "searxng".to_string(),
            results: vec![],
        };
    };
    let Ok(payload) = response.json::<SearxngResponse>().await else {
        return ProviderResponse {
            provider: "searxng".to_string(),
            results: vec![],
        };
    };
    let results = payload
        .results
        .into_iter()
        .filter_map(|item| {
            Some(ProviderResult {
                url: item.url?,
                title: item.title,
                description: item.content,
            })
        })
        .take(limit)
        .collect();
    ProviderResponse {
        provider: "searxng".to_string(),
        results,
    }
}

#[allow(clippy::too_many_arguments)]
async fn search_duckduckgo(
    client: &reqwest::Client,
    query: &str,
    limit: usize,
    lang: &str,
    country: &str,
    tbs: Option<&str>,
    location: Option<&str>,
    filter: Option<&str>,
) -> ProviderResponse {
    let response = client
        .get("https://html.duckduckgo.com/html")
        .query(&[
            ("q", query),
            ("kp", if filter == Some("off") { "-2" } else { "1" }),
            (
                "kl",
                &format!("{}-{}", country.to_lowercase(), lang.to_lowercase()),
            ),
            ("df", tbs.unwrap_or_default()),
            ("location", location.unwrap_or_default()),
        ])
        .header(USER_AGENT, DEFAULT_USER_AGENT)
        .header(
            ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(ACCEPT_LANGUAGE, "en-US,en;q=0.5")
        .send()
        .await;
    let Ok(response) = response else {
        return ProviderResponse {
            provider: "duckduckgo".to_string(),
            results: vec![],
        };
    };
    let Ok(html) = response.text().await else {
        return ProviderResponse {
            provider: "duckduckgo".to_string(),
            results: vec![],
        };
    };
    let document = Html::parse_document(&html);
    let result_selector = Selector::parse(".result").unwrap();
    let link_selector = Selector::parse(".result__a").unwrap();
    let snippet_selector = Selector::parse(".result__snippet").unwrap();
    let mut results = Vec::new();
    for result in document.select(&result_selector) {
        let Some(link) = result.select(&link_selector).next() else {
            continue;
        };
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        let url = decode_duckduckgo_url(href);
        if url.is_empty() {
            continue;
        }
        let title = clean_text(&link.text().collect::<Vec<_>>().join(" "));
        let description = result
            .select(&snippet_selector)
            .next()
            .map(|node| clean_text(&node.text().collect::<Vec<_>>().join(" ")));
        results.push(ProviderResult {
            url,
            title: (!title.is_empty()).then_some(title),
            description,
        });
        if results.len() >= limit {
            break;
        }
    }
    ProviderResponse {
        provider: "duckduckgo".to_string(),
        results,
    }
}

fn decode_duckduckgo_url(href: &str) -> String {
    if let Ok(parsed) = Url::parse(href) {
        if parsed.host_str().unwrap_or("").ends_with("duckduckgo.com")
            || parsed.path().starts_with("/l/")
        {
            for (key, value) in parsed.query_pairs() {
                if key == "uddg" {
                    return value.to_string();
                }
            }
        }
    }
    href.to_string()
}

fn clean_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn build_search_query(
    query: &str,
    categories: &[crate::models::SearchCategory],
    include_domains: &[String],
    exclude_domains: &[String],
) -> String {
    let mut filters = Vec::new();
    let mut pdf = false;
    for category in categories {
        match category.name.as_str() {
            "github" => filters.push("site:github.com".to_string()),
            "research" => {
                if category.sites.is_empty() {
                    filters.extend(
                        ["arxiv.org", "pubmed.ncbi.nlm.nih.gov", "nature.com"]
                            .iter()
                            .map(|site| format!("site:{site}")),
                    );
                } else {
                    filters.extend(category.sites.iter().map(|site| format!("site:{site}")));
                }
            }
            "pdf" => pdf = true,
            _ => {}
        }
    }
    filters.extend(
        include_domains
            .iter()
            .map(|domain| format!("site:{domain}")),
    );
    let mut built = query.to_string();
    if !filters.is_empty() {
        built.push_str(" (");
        built.push_str(&filters.join(" OR "));
        built.push(')');
    }
    for domain in exclude_domains {
        built.push_str(&format!(" -site:{domain}"));
    }
    if pdf {
        built.push_str(" filetype:pdf");
    }
    built
}

fn domain_allowed(url: &str, include: &[String], exclude: &[String]) -> bool {
    let host = Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    let matches = |domain: &String| {
        let domain = domain.trim_start_matches("www.").to_ascii_lowercase();
        host == domain || host.ends_with(&format!(".{domain}"))
    };
    (include.is_empty() || include.iter().any(matches)) && !exclude.iter().any(matches)
}

fn category_for_url(url: &str, categories: &[crate::models::SearchCategory]) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    for category in categories {
        if category.name == "pdf" && parsed.path().to_ascii_lowercase().ends_with(".pdf") {
            return Some("pdf".to_string());
        }
        if category.name == "github" && (host == "github.com" || host.ends_with(".github.com")) {
            return Some("github".to_string());
        }
        if category.name == "research"
            && (category.sites.iter().any(|site| host.ends_with(site))
                || ["arxiv.org", "pubmed.ncbi.nlm.nih.gov", "nature.com"]
                    .iter()
                    .any(|site| host.ends_with(site)))
        {
            return Some("research".to_string());
        }
    }
    None
}

fn query_highlights(text: &str, query: &str) -> Vec<String> {
    let terms = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.len() > 2)
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    text.split("\n\n")
        .filter(|passage| {
            let lower = passage.to_ascii_lowercase();
            terms.iter().any(|term| lower.contains(term))
        })
        .map(|passage| passage.trim().chars().take(500).collect::<String>())
        .filter(|passage| !passage.is_empty())
        .take(5)
        .collect()
}

#[derive(Debug, Deserialize)]
struct NewsRss {
    channel: NewsChannel,
}

#[derive(Debug, Deserialize)]
struct NewsChannel {
    #[serde(default)]
    item: Vec<NewsItem>,
}

#[derive(Debug, Deserialize)]
struct NewsItem {
    title: Option<String>,
    link: Option<String>,
    description: Option<String>,
    #[serde(rename = "pubDate")]
    published_at: Option<String>,
}

pub async fn search_news(
    client: &reqwest::Client,
    query: &str,
    limit: usize,
    lang: &str,
    country: &str,
    tbs: Option<&str>,
) -> Vec<Value> {
    let response = client
        .get("https://www.bing.com/news/search")
        .query(&[
            ("q", query),
            ("format", "rss"),
            ("setlang", lang),
            ("cc", country),
            ("qft", tbs.unwrap_or_default()),
        ])
        .header(USER_AGENT, DEFAULT_USER_AGENT)
        .send()
        .await;
    let Ok(xml) = response.and_then(|response| response.error_for_status()) else {
        return vec![];
    };
    let Ok(xml) = xml.text().await else {
        return vec![];
    };
    parse_news_rss(&xml, limit)
}

fn parse_news_rss(xml: &str, limit: usize) -> Vec<Value> {
    let Ok(feed) = quick_xml::de::from_str::<NewsRss>(xml) else {
        return vec![];
    };
    feed.channel
        .item
        .into_iter()
        .filter_map(|item| {
            Some(serde_json::json!({
                "title": item.title?,
                "url": item.link?,
                "snippet": item.description.map(|value| clean_text(&html_escape::decode_html_entities(&value))),
                "date": item.published_at,
            }))
        })
        .take(limit)
        .collect()
}

pub async fn search_images(client: &reqwest::Client, query: &str, limit: usize) -> Vec<Value> {
    let response = client
        .get("https://www.bing.com/images/search")
        .query(&[("q", query), ("count", &limit.to_string())])
        .header(USER_AGENT, DEFAULT_USER_AGENT)
        .send()
        .await;
    let Ok(response) = response.and_then(|response| response.error_for_status()) else {
        return vec![];
    };
    let Ok(html) = response.text().await else {
        return vec![];
    };
    parse_image_html(&html, limit)
}

fn parse_image_html(html: &str, limit: usize) -> Vec<Value> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("a.iusc").expect("static selector");
    document
        .select(&selector)
        .filter_map(|node| node.value().attr("m"))
        .filter_map(|metadata| serde_json::from_str::<Value>(metadata).ok())
        .filter_map(|metadata| {
            Some(serde_json::json!({
                "title": metadata.get("t").and_then(Value::as_str),
                "imageUrl": metadata.get("murl")?.as_str()?,
                "url": metadata.get("purl").and_then(Value::as_str),
                "width": metadata.get("w"),
                "height": metadata.get("h"),
            }))
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SearchCategory;

    #[test]
    fn search_query_combines_categories_and_domain_filters() {
        let query = build_search_query(
            "rust crawler",
            &[
                SearchCategory {
                    name: "github".to_string(),
                    sites: vec![],
                },
                SearchCategory {
                    name: "pdf".to_string(),
                    sites: vec![],
                },
            ],
            &["example.com".to_string()],
            &[],
        );
        assert!(query.contains("site:github.com"));
        assert!(query.contains("site:example.com"));
        assert!(query.contains("filetype:pdf"));
        assert!(domain_allowed(
            "https://docs.example.com/page",
            &["example.com".to_string()],
            &[]
        ));
    }

    #[test]
    fn parses_news_and_image_provider_payloads() {
        let news = parse_news_rss(
            "<rss><channel><item><title>Bee News</title><link>https://example.com/news</link><description>Useful &amp; timely</description><pubDate>today</pubDate></item></channel></rss>",
            10,
        );
        assert_eq!(news[0]["title"], "Bee News");
        assert_eq!(news[0]["snippet"], "Useful & timely");

        let images = parse_image_html(
            r#"<a class="iusc" m='{"t":"Bee","murl":"https://img.example/bee.png","purl":"https://example.com"}'></a>"#,
            10,
        );
        assert_eq!(images[0]["imageUrl"], "https://img.example/bee.png");
    }

    #[test]
    fn highlights_select_query_relevant_passages() {
        let highlights = query_highlights(
            "General intro.\n\nBeeCrawl asynchronously scrapes search results.\n\nOther text.",
            "async scrape",
        );
        assert_eq!(
            highlights,
            ["BeeCrawl asynchronously scrapes search results."]
        );
    }
}
