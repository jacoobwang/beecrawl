use std::collections::HashMap;
use std::time::Instant;

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
    let provider = search_web(
        client,
        &request.query,
        request.limit,
        &request.lang,
        &request.country,
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
        })
        .collect();

    let should_scrape = request
        .scrape_options
        .as_ref()
        .map(|options| !options.formats.is_empty())
        .unwrap_or(false);
    if should_scrape {
        let options = request.scrape_options.clone().unwrap();
        for result in &mut results {
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
                },
            )
            .await
            {
                Ok(scrape) => {
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

async fn search_web(
    client: &reqwest::Client,
    query: &str,
    limit: usize,
    lang: &str,
    country: &str,
) -> ProviderResponse {
    if let Ok(endpoint) = std::env::var("BEECRAWL_SEARXNG_ENDPOINT") {
        if !endpoint.trim().is_empty() {
            let response = search_searxng(client, endpoint.trim(), query, limit, lang).await;
            if !response.results.is_empty() {
                return response;
            }
        }
    }
    search_duckduckgo(client, query, limit, lang, country).await
}

async fn search_searxng(
    client: &reqwest::Client,
    endpoint: &str,
    query: &str,
    limit: usize,
    lang: &str,
) -> ProviderResponse {
    let url = format!("{}/search", endpoint.trim_end_matches('/'));
    let response = client
        .get(url)
        .query(&[("q", query), ("language", lang), ("format", "json")])
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

async fn search_duckduckgo(
    client: &reqwest::Client,
    query: &str,
    limit: usize,
    lang: &str,
    country: &str,
) -> ProviderResponse {
    let response = client
        .get("https://html.duckduckgo.com/html")
        .query(&[
            ("q", query),
            ("kp", "1"),
            (
                "kl",
                &format!("{}-{}", country.to_lowercase(), lang.to_lowercase()),
            ),
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
