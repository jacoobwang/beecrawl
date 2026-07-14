use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Multipart, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tower_http::trace::TraceLayer;

use crate::models::{
    BatchScrapeEnqueueResponse, BatchScrapeRequest, CrawlEnqueueResponse, CrawlRequest,
    CrawlStatusQuery, CrawlStatusResponse, ExtractMetadata, ExtractRequest, ExtractResponse,
    FirecrawlV2CrawlRequest, FirecrawlV2ExtractRequest, FirecrawlV2ParseOptions,
    FirecrawlV2ScrapeRequest, FirecrawlV2SearchRequest, Link, ScrapeResponse, SearchRequest,
    SearchScrapeOptions, WebExtractMapRequest, WebExtractScrapeRequest, WebExtractScrapeResponse,
};
use crate::{
    cache::CacheStore,
    crawl::{CrawlStore, CrawlStoreError},
    llm, search, web_extract,
};

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    cache: CacheStore,
    crawls: CrawlStore,
}

pub fn app() -> Router {
    app_with_crawls(CrawlStore::from_env())
}

fn app_with_crawls(crawls: CrawlStore) -> Router {
    let state = AppState {
        client: reqwest::Client::new(),
        cache: CacheStore::from_env(),
        crawls,
    };
    Router::new()
        .route("/health", get(health))
        .route("/scrape", post(scrape))
        .route("/crawl", post(crawl))
        .route("/crawl/:id", get(crawl_status).delete(cancel_crawl))
        .route("/batch/scrape", post(batch_scrape))
        .route("/batch/scrape/:id", get(crawl_status).delete(cancel_crawl))
        .route("/map", post(map_site))
        .route("/search", post(search_route))
        .route("/extract", post(extract))
        .route("/v2/scrape", post(firecrawl_v2_scrape))
        .route("/v2/parse", post(firecrawl_v2_parse))
        .route("/v2/crawl", post(firecrawl_v2_crawl))
        .route(
            "/v2/crawl/:id",
            get(firecrawl_v2_crawl_status).delete(firecrawl_v2_cancel_crawl),
        )
        .route("/v2/map", post(firecrawl_v2_map))
        .route("/v2/extract", post(firecrawl_v2_extract))
        .route("/v2/search", post(firecrawl_v2_search))
        .layer(TraceLayer::new_for_http())
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024))
        .with_state(Arc::new(state))
}

async fn crawl(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CrawlRequest>,
) -> Result<Json<CrawlEnqueueResponse>, ApiError> {
    require_auth(&headers)?;
    state
        .crawls
        .enqueue(request)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn batch_scrape(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<BatchScrapeRequest>,
) -> Result<Json<BatchScrapeEnqueueResponse>, ApiError> {
    require_auth(&headers)?;
    state
        .crawls
        .enqueue_batch(request)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn crawl_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Query(query): Query<CrawlStatusQuery>,
) -> Result<Json<CrawlStatusResponse>, ApiError> {
    require_auth(&headers)?;
    state
        .crawls
        .get(&id, query)
        .await
        .map_err(ApiError::from)?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

async fn cancel_crawl(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<CrawlStatusResponse>, ApiError> {
    require_auth(&headers)?;
    state
        .crawls
        .cancel(&id)
        .await
        .map_err(ApiError::from)?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true }))
}

async fn scrape(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<WebExtractScrapeRequest>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let response = web_extract::scrape_with_cache(&state.client, &state.cache, request).await?;
    Ok(Json(response).into_response())
}

async fn map_site(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<WebExtractMapRequest>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let response = web_extract::map_site(&state.client, request).await?;
    Ok(Json(response).into_response())
}

async fn search_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<crate::models::SearchRequest>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let response = search::search(&state.client, request)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(Json(response).into_response())
}

async fn extract(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ExtractRequest>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    Ok(Json(run_extract(&state, request).await?).into_response())
}

async fn run_extract(
    state: &AppState,
    request: ExtractRequest,
) -> Result<ExtractResponse, ApiError> {
    let scrape_response = web_extract::scrape_with_cache(
        &state.client,
        &state.cache,
        WebExtractScrapeRequest {
            url: request.url.clone(),
            formats: vec!["markdown".to_string()],
            location: None,
            timeout_seconds: request.timeout_seconds,
            wait_for_ms: request.wait_for_ms,
            use_browser: request.use_browser.clone(),
        },
    )
    .await?;
    let text = scrape_response.markdown;
    let provider_override = request.provider.as_ref().or(request.llm.as_ref());
    let llm_provider = llm::resolve_provider(provider_override)?;
    let (data, extract_provider, extract_model) = if let Some(provider) = llm_provider {
        (
            llm::extract_structured_data(
                &state.client,
                &provider,
                &request.url,
                &request.schema,
                &text,
            )
            .await?,
            provider.provider,
            Some(provider.model),
        )
    } else {
        let mut data = HashMap::new();
        for field in request.schema.keys() {
            data.insert(
                field.clone(),
                extract_field(field, &text, scrape_response.metadata.title.clone()),
            );
        }
        (data, "deterministic".to_string(), None)
    };
    let scrape = ScrapeResponse {
        url: request.url.clone(),
        title: scrape_response.metadata.title,
        text,
        links: vec![],
        metadata: HashMap::from([
            ("provider".to_string(), scrape_response.metadata.provider),
            (
                "rendered".to_string(),
                scrape_response.metadata.rendered.to_string(),
            ),
        ]),
    };
    Ok(ExtractResponse {
        url: request.url,
        data,
        scrape,
        metadata: ExtractMetadata {
            provider: extract_provider,
            model: extract_model,
        },
    })
}

async fn firecrawl_v2_scrape(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<FirecrawlV2ScrapeRequest>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let response = web_extract::scrape_with_cache(
        &state.client,
        &state.cache,
        WebExtractScrapeRequest {
            url: request.url,
            formats: request.formats,
            location: request.location,
            timeout_seconds: request.timeout.div_ceil(1_000).max(1),
            wait_for_ms: request.wait_for_ms,
            use_browser: "auto".to_string(),
        },
    )
    .await?;
    Ok(Json(json!({ "success": true, "data": firecrawl_document(response) })).into_response())
}

async fn firecrawl_v2_parse(
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let mut file = None;
    let mut filename = None;
    let mut options = FirecrawlV2ParseOptions {
        formats: vec!["markdown".to_string()],
        timeout: 30_000,
        parsers: Vec::new(),
    };

    while let Some(field) = multipart.next_field().await.map_err(|error| {
        ApiError::InvalidRequest(format!("Invalid multipart form-data request: {error}"))
    })? {
        match field.name() {
            Some("file") => {
                filename = field.file_name().map(str::to_string);
                file = Some(field.bytes().await.map_err(|error| {
                    ApiError::InvalidRequest(format!("Could not read uploaded file: {error}"))
                })?);
            }
            Some("options") => {
                let value = field.text().await.map_err(|error| {
                    ApiError::InvalidRequest(format!("Could not read parse options: {error}"))
                })?;
                options = serde_json::from_str(&value).map_err(|error| {
                    ApiError::InvalidRequest(format!("Invalid parse options JSON: {error}"))
                })?;
            }
            _ => {}
        }
    }

    let filename =
        filename.ok_or_else(|| ApiError::InvalidRequest("Missing file field".to_string()))?;
    if !filename.to_ascii_lowercase().ends_with(".pdf") {
        return Err(ApiError::InvalidRequest(
            "Only PDF files are currently supported by /v2/parse".to_string(),
        ));
    }
    let bytes = file.ok_or_else(|| ApiError::InvalidRequest("Missing file field".to_string()))?;
    if !bytes.starts_with(b"%PDF-") {
        return Err(ApiError::InvalidRequest(
            "Uploaded file is not a PDF".to_string(),
        ));
    }
    if options
        .formats
        .iter()
        .any(|format| !format.eq_ignore_ascii_case("markdown"))
    {
        return Err(ApiError::InvalidRequest(
            "Only the markdown format is currently supported by /v2/parse".to_string(),
        ));
    }
    if options.timeout > 300_000 {
        return Err(ApiError::InvalidRequest(
            "Parse timeout must not exceed 300000 milliseconds".to_string(),
        ));
    }
    let parser = options
        .parsers
        .iter()
        .find(|parser| parser.kind.eq_ignore_ascii_case("pdf"));
    if options
        .parsers
        .iter()
        .any(|parser| !parser.kind.eq_ignore_ascii_case("pdf"))
    {
        return Err(ApiError::InvalidRequest(
            "Unsupported file parser".to_string(),
        ));
    }
    if let Some(mode) = parser.and_then(|parser| parser.mode.as_deref()) {
        if !matches!(mode, "fast" | "auto") {
            return Err(ApiError::InvalidRequest(
                "OCR PDF parsing is not currently supported".to_string(),
            ));
        }
    }
    let max_pages = parser.and_then(|parser| parser.max_pages);
    if matches!(max_pages, Some(0)) || max_pages.is_some_and(|value| value > 10_000) {
        return Err(ApiError::InvalidRequest(
            "PDF maxPages must be between 1 and 10000".to_string(),
        ));
    }
    let parsed = tokio::task::spawn_blocking(move || crate::pdf::parse(&bytes, max_pages))
        .await
        .map_err(|error| ApiError::Internal(format!("PDF parser failed: {error}")))?
        .map_err(ApiError::InvalidRequest)?;
    Ok(Json(json!({
        "success": true,
        "data": {
            "markdown": parsed.markdown,
            "metadata": {
                "numPages": parsed.num_pages,
                "totalPages": parsed.total_pages,
                "sourceFile": filename,
            }
        }
    }))
    .into_response())
}

async fn firecrawl_v2_map(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<WebExtractMapRequest>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let response = web_extract::map_site(&state.client, request).await?;
    Ok(Json(json!({ "success": true, "links": response.links })).into_response())
}

async fn firecrawl_v2_crawl(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<FirecrawlV2CrawlRequest>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let scrape = request.scrape_options.unwrap_or_default();
    let response = state
        .crawls
        .enqueue(CrawlRequest {
            url: request.url,
            limit: request.limit,
            max_depth: request.max_discovery_depth,
            include_subdomains: request.allow_subdomains,
            ignore_query_parameters: request.ignore_query_parameters,
            timeout_seconds: scrape.timeout.div_ceil(1_000).max(1),
            wait_for_ms: scrape.wait_for_ms,
            use_browser: "auto".to_string(),
            max_retries: 2,
        })
        .await?;
    Ok(Json(json!({ "success": true, "id": response.id, "url": response.url })).into_response())
}

async fn firecrawl_v2_crawl_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Query(query): Query<CrawlStatusQuery>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let response = state
        .crawls
        .get(&id, query)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(firecrawl_crawl_status(response)).into_response())
}

async fn firecrawl_v2_cancel_crawl(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let response = state.crawls.cancel(&id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(json!({
        "success": true,
        "status": firecrawl_status(&response.status),
    }))
    .into_response())
}

async fn firecrawl_v2_extract(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<FirecrawlV2ExtractRequest>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    if request.urls.is_empty() {
        return Err(ApiError::InvalidRequest(
            "Firecrawl v2 extract requires at least one URL".to_string(),
        ));
    }
    let mut pages = Vec::with_capacity(request.urls.len());
    for url in &request.urls {
        pages.push(
            web_extract::scrape_with_cache(
                &state.client,
                &state.cache,
                WebExtractScrapeRequest {
                    url: url.clone(),
                    formats: vec!["markdown".to_string()],
                    location: None,
                    timeout_seconds: 30,
                    wait_for_ms: 0,
                    use_browser: "auto".to_string(),
                },
            )
            .await?,
        );
    }
    let markdown = pages
        .iter()
        .map(|page| format!("# Source: {}\n\n{}", page.final_url, page.markdown))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    let provider = llm::resolve_provider(None)?;
    let data = if let Some(provider) = provider {
        llm::extract_structured_value(
            &state.client,
            &provider,
            &request.urls,
            &request.schema,
            &markdown,
            request.prompt.as_deref(),
        )
        .await?
    } else {
        let simple_schema = firecrawl_extract_schema(&request.schema);
        serde_json::to_value(
            simple_schema
                .keys()
                .map(|field| {
                    (
                        field.clone(),
                        extract_field(field, &markdown, pages[0].metadata.title.clone()),
                    )
                })
                .collect::<HashMap<_, _>>(),
        )
        .unwrap_or_else(|_| json!({}))
    };
    let sources = request.show_sources.then(|| {
        json!({
            "urls": request.urls,
        })
    });
    let _ = request.enable_web_search;
    Ok(Json(json!({
        "success": true,
        "status": "completed",
        "data": data,
        "sources": sources,
    }))
    .into_response())
}

async fn firecrawl_v2_search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<FirecrawlV2SearchRequest>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    if request.query.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "Firecrawl v2 search query cannot be empty".to_string(),
        ));
    }
    if !(1..=100).contains(&request.limit) {
        return Err(ApiError::InvalidRequest(
            "Firecrawl v2 search limit must be between 1 and 100".to_string(),
        ));
    }

    let requested_sources = request
        .sources
        .iter()
        .map(|source| source.name())
        .collect::<Vec<_>>();
    let wants_web = requested_sources.is_empty() || requested_sources.contains(&"web");
    let wants_news = requested_sources.contains(&"news");
    let wants_images = requested_sources.contains(&"images");

    if !wants_web {
        let mut data = serde_json::Map::new();
        if wants_news {
            data.insert("news".to_string(), json!([]));
        }
        if wants_images {
            data.insert("images".to_string(), json!([]));
        }
        return Ok(Json(json!({ "success": true, "data": data })).into_response());
    }

    let response = search::search(
        &state.client,
        SearchRequest {
            query: request.query,
            limit: request.limit,
            lang: "en".to_string(),
            country: "us".to_string(),
            scrape_options: request.scrape_options.map(|options| SearchScrapeOptions {
                formats: options.formats,
                timeout_seconds: options.timeout.div_ceil(1_000).max(1),
                wait_for_ms: options.wait_for_ms,
                use_browser: "auto".to_string(),
            }),
        },
    )
    .await
    .map_err(|error| ApiError::Internal(error.to_string()))?;

    let web = response
        .results
        .into_iter()
        .map(firecrawl_search_result)
        .collect::<Vec<_>>();
    let mut data = serde_json::Map::new();
    data.insert("web".to_string(), json!(web));
    if wants_news {
        data.insert("news".to_string(), json!([]));
    }
    if wants_images {
        data.insert("images".to_string(), json!([]));
    }
    Ok(Json(json!({ "success": true, "data": data })).into_response())
}

fn firecrawl_search_result(result: crate::models::SearchResult) -> serde_json::Value {
    if let Some(markdown) = result.markdown {
        json!({
            "url": result.url,
            "title": result.title,
            "description": result.description,
            "markdown": markdown,
            "metadata": {
                "title": result.title,
                "description": result.description,
                "sourceURL": result.url,
                "url": result.metadata.get("final_url"),
            }
        })
    } else {
        json!({
            "url": result.url,
            "title": result.title,
            "description": result.description,
        })
    }
}

fn firecrawl_document(response: WebExtractScrapeResponse) -> serde_json::Value {
    json!({
        "markdown": response.markdown,
        "html": response.html,
        "rawHtml": response.raw_html,
        "links": response.links,
        "screenshot": response.screenshot,
        "metadata": {
            "title": response.metadata.title,
            "language": response.metadata.language,
            "sourceURL": response.url,
            "url": response.final_url,
            "statusCode": response.metadata.status_code,
            "scrapeId": response.request_id,
        }
    })
}

fn firecrawl_crawl_status(response: CrawlStatusResponse) -> serde_json::Value {
    json!({
        "success": true,
        "status": firecrawl_status(&response.status),
        "total": response.total,
        "completed": response.completed,
        "creditsUsed": response.completed,
        "data": response.data.into_iter().map(firecrawl_document).collect::<Vec<_>>(),
        "next": serde_json::Value::Null,
    })
}

fn firecrawl_status(status: &str) -> &str {
    match status {
        "queued" | "running" => "scraping",
        other => other,
    }
}

fn firecrawl_extract_schema(schema: &serde_json::Value) -> HashMap<String, String> {
    let properties = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .or_else(|| schema.as_object());
    properties
        .into_iter()
        .flatten()
        .map(|(name, definition)| {
            let description = definition
                .get("description")
                .and_then(serde_json::Value::as_str)
                .or_else(|| definition.get("type").and_then(serde_json::Value::as_str))
                .unwrap_or("string")
                .to_string();
            (name.clone(), description)
        })
        .collect()
}

fn extract_field(field: &str, text: &str, title: Option<String>) -> Option<String> {
    let field_lower = field.to_lowercase();
    if field_lower.contains("title")
        || field_lower.contains("company")
        || field_lower.contains("name")
    {
        if let Some(title) = title {
            return Some(title);
        }
    }
    if field_lower.contains("email") {
        return text
            .split_whitespace()
            .find(|part| part.contains('@') && part.contains('.'))
            .map(|part| {
                part.trim_matches(|c: char| !c.is_alphanumeric() && c != '@' && c != '.')
                    .to_string()
            });
    }
    None
}

fn require_auth(headers: &HeaderMap) -> Result<(), ApiError> {
    let api_key = std::env::var("BEECRAWL_WEB_EXTRACT_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("WEB_EXTRACT_API_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty())
        });
    let Some(api_key) = api_key else {
        return Ok(());
    };
    let supplied = headers
        .get("x-web-extract-api-key")
        .or_else(|| headers.get("x-api-key"))
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .map(str::to_string)
        });
    if supplied.as_deref() == Some(api_key.as_str()) {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

#[derive(Debug)]
pub enum ApiError {
    WebExtract(web_extract::WebExtractError),
    Crawl(CrawlStoreError),
    Llm(llm::LlmError),
    Unauthorized,
    InvalidRequest(String),
    NotFound,
    Internal(String),
}

impl From<web_extract::WebExtractError> for ApiError {
    fn from(value: web_extract::WebExtractError) -> Self {
        Self::WebExtract(value)
    }
}

impl From<CrawlStoreError> for ApiError {
    fn from(value: CrawlStoreError) -> Self {
        Self::Crawl(value)
    }
}

impl From<llm::LlmError> for ApiError {
    fn from(value: llm::LlmError) -> Self {
        Self::Llm(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Crawl(error) => (
                error.status(),
                Json(json!({
                    "detail": {
                        "code": error.code(),
                        "message": error.to_string(),
                        "retryable": matches!(error, CrawlStoreError::StorageUnavailable(_) | CrawlStoreError::Database(_))
                    }
                })),
            )
                .into_response(),
            Self::WebExtract(error) => (
                error.status(),
                Json(json!({
                    "detail": {
                        "code": error.code(),
                        "message": error.to_string(),
                        "retryable": matches!(error, web_extract::WebExtractError::FetchFailed(_) | web_extract::WebExtractError::RenderFailed(_))
                    }
                })),
            )
                .into_response(),
            Self::Llm(error) => (
                error.status(),
                Json(json!({
                    "detail": {
                        "code": error.code(),
                        "message": error.to_string(),
                        "retryable": matches!(error, llm::LlmError::RequestFailed(_))
                    }
                })),
            )
                .into_response(),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "detail": {
                        "code": "unauthorized",
                        "message": "Invalid web extraction API key",
                        "retryable": false
                    }
                })),
            )
                .into_response(),
            Self::InvalidRequest(message) => (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "error": message,
                })),
            )
                .into_response(),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "detail": {
                        "code": "crawl_not_found",
                        "message": "Crawl job not found",
                        "retryable": false
                    }
                })),
            )
                .into_response(),
            Self::Internal(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "detail": message })),
            )
                .into_response(),
        }
    }
}

#[allow(dead_code)]
fn _link(text: String, url: String) -> Link {
    Link { text, url }
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn old_scrape_paths_are_not_registered() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/scrape")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"url":"https://example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn crawl_routes_require_postgres() {
        let app = app_with_crawls(CrawlStore::unavailable("Postgres is not configured"));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/crawl")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"url":"https://example.invalid","limit":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["detail"]["code"], "crawl_storage_unavailable");
    }

    #[test]
    fn firecrawl_v2_crawl_accepts_camel_case_options() {
        let request: FirecrawlV2CrawlRequest = serde_json::from_value(json!({
            "url": "https://example.com",
            "limit": 12,
            "maxDiscoveryDepth": 4,
            "allowSubdomains": true,
            "ignoreQueryParameters": false,
            "scrapeOptions": { "formats": ["markdown"], "waitFor": 250, "timeout": 45000 }
        }))
        .unwrap();
        assert_eq!(request.max_discovery_depth, 4);
        assert!(request.allow_subdomains);
        let scrape = request.scrape_options.unwrap();
        assert_eq!(scrape.wait_for_ms, 250);
        assert_eq!(scrape.timeout, 45_000);
    }

    #[test]
    fn firecrawl_v2_extract_keeps_json_schema_properties() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": ["string", "null"] },
                "references": { "type": "array", "items": { "type": "object" } }
            }
        });
        let fields = firecrawl_extract_schema(&schema);
        assert!(fields.contains_key("name"));
        assert!(fields.contains_key("references"));
        assert!(!fields.contains_key("type"));
    }

    #[test]
    fn queued_crawls_are_reported_as_firecrawl_scraping() {
        assert_eq!(firecrawl_status("queued"), "scraping");
        assert_eq!(firecrawl_status("completed"), "completed");
    }

    #[test]
    fn firecrawl_v2_search_accepts_source_names_and_objects() {
        let request: FirecrawlV2SearchRequest = serde_json::from_value(json!({
            "query": "thermal insulation",
            "sources": ["web", { "type": "news" }],
            "scrapeOptions": { "formats": [{ "type": "markdown" }], "timeout": 45000 }
        }))
        .unwrap();
        assert_eq!(request.sources[0].name(), "web");
        assert_eq!(request.sources[1].name(), "news");
        let scrape_options = request.scrape_options.unwrap();
        assert_eq!(scrape_options.formats, ["markdown"]);
        assert_eq!(scrape_options.timeout, 45_000);
    }

    #[test]
    fn firecrawl_v2_formats_accept_strings_and_objects() {
        let request: FirecrawlV2ScrapeRequest = serde_json::from_value(json!({
            "url": "https://example.com",
            "formats": ["html", { "type": "markdown" }, { "type": "screenshot", "fullPage": true }]
        }))
        .unwrap();
        assert_eq!(request.formats, ["html", "markdown", "screenshot"]);
    }

    #[test]
    fn firecrawl_v2_parse_accepts_pdf_parser_options() {
        let options: FirecrawlV2ParseOptions = serde_json::from_value(json!({
            "formats": [{ "type": "markdown" }],
            "timeout": 120000,
            "parsers": [{ "type": "pdf", "mode": "fast", "maxPages": 12 }]
        }))
        .unwrap();
        assert_eq!(options.formats, ["markdown"]);
        assert_eq!(options.timeout, 120_000);
        assert_eq!(options.parsers[0].kind, "pdf");
        assert_eq!(options.parsers[0].mode.as_deref(), Some("fast"));
        assert_eq!(options.parsers[0].max_pages, Some(12));
    }

    #[test]
    fn firecrawl_v2_scraped_search_result_is_a_document() {
        let result = firecrawl_search_result(crate::models::SearchResult {
            url: "https://example.com".to_string(),
            title: Some("Example".to_string()),
            description: Some("Description".to_string()),
            markdown: Some("# Example".to_string()),
            metadata: HashMap::from([("final_url".to_string(), json!("https://www.example.com/"))]),
            scrape_error: None,
        });
        assert_eq!(result["markdown"], "# Example");
        assert_eq!(result["title"], "Example");
        assert_eq!(result["description"], "Description");
        assert_eq!(result["url"], "https://example.com");
        assert_eq!(result["metadata"]["sourceURL"], "https://example.com");
    }
}
