use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tower_http::trace::TraceLayer;

use crate::models::{
    ExtractMetadata, ExtractRequest, ExtractResponse, Link, ScrapeResponse, WebExtractMapRequest,
    WebExtractScrapeRequest,
};
use crate::{llm, search, web_extract};

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
}

pub fn app() -> Router {
    let state = AppState {
        client: reqwest::Client::new(),
    };
    Router::new()
        .route("/health", get(health))
        .route("/scrape", post(scrape))
        .route("/map", post(map_site))
        .route("/search", post(search_route))
        .route("/extract", post(extract))
        .layer(TraceLayer::new_for_http())
        .with_state(Arc::new(state))
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
    let response = web_extract::scrape(&state.client, request).await?;
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
    let scrape_response = web_extract::scrape(
        &state.client,
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
    Llm(llm::LlmError),
    Unauthorized,
    Internal(String),
}

impl From<web_extract::WebExtractError> for ApiError {
    fn from(value: web_extract::WebExtractError) -> Self {
        Self::WebExtract(value)
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
    use axum::body::Body;
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
}
