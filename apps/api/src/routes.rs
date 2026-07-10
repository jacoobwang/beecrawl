use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tower_http::trace::TraceLayer;

use crate::models::{
    BatchScrapeEnqueueResponse, BatchScrapeRequest, CrawlEnqueueResponse, CrawlRequest,
    CrawlStatusQuery, CrawlStatusResponse, ExtractMetadata, ExtractRequest, ExtractResponse, Link,
    ScrapeResponse, WebExtractMapRequest, WebExtractScrapeRequest,
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
        .layer(TraceLayer::new_for_http())
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
    Json(request): Json<ExtractRequest>,
) -> Result<Response, ApiError> {
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
    Ok(Json(ExtractResponse {
        url: request.url,
        data,
        scrape,
        metadata: ExtractMetadata {
            provider: extract_provider,
            model: extract_model,
        },
    })
    .into_response())
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
}
