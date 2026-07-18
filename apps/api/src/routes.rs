use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::rejection::JsonRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use base64::Engine as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower_http::trace::TraceLayer;

use crate::models::{
    BatchScrapeEnqueueResponse, BatchScrapeRequest, ContentOptions, CrawlEnqueueResponse,
    CrawlRequest, CrawlStatusQuery, CrawlStatusResponse, ExtractMetadata, ExtractRequest,
    ExtractResponse, FirecrawlFormat, FirecrawlV2Base64ParseRequest, FirecrawlV2BatchScrapeRequest,
    FirecrawlV2CrawlRequest, FirecrawlV2ExtractRequest, FirecrawlV2MapRequest,
    FirecrawlV2ParseOptions, FirecrawlV2ScrapeRequest, FirecrawlV2SearchRequest, Link, ProxyConfig,
    ScrapeResponse, ScreenshotOptions, ScreenshotViewport, SearchRequest, SearchScrapeOptions,
    WebExtractMapRequest, WebExtractScrapeRequest, WebExtractScrapeResponse,
};
use crate::{
    cache::CacheStore,
    crawl::{CrawlStore, CrawlStoreError},
    llm, search, web_extract, webhook,
    workflows::{
        self, AgentCreateRequest, MonitorCreateRequest, MonitorUpdateRequest, WorkflowStore,
    },
};

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    cache: CacheStore,
    crawls: CrawlStore,
    workflows: WorkflowStore,
    parse_uploads: Arc<tokio::sync::Mutex<HashMap<String, ParseUpload>>>,
    browser_owners: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
    scrape_handoffs: Arc<tokio::sync::Mutex<HashMap<String, ScrapeHandoff>>>,
}

#[derive(Clone)]
struct ParseUpload {
    filename: String,
    expires_at: chrono::DateTime<chrono::Utc>,
    data: Option<Bytes>,
}

#[derive(Clone)]
struct ScrapeHandoff {
    url: String,
    storage_state: Option<Value>,
    created_at: chrono::DateTime<chrono::Utc>,
    session_id: Option<String>,
}

pub fn app() -> Router {
    app_with_crawls(CrawlStore::from_env())
}

fn app_with_crawls(crawls: CrawlStore) -> Router {
    let state = AppState {
        client: reqwest::Client::new(),
        cache: CacheStore::from_env(),
        crawls,
        workflows: WorkflowStore::from_env(),
        parse_uploads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        browser_owners: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        scrape_handoffs: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
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
        .route("/v2/parse/base64", post(firecrawl_v2_parse_base64))
        .route("/v2/parse/reference", post(firecrawl_v2_parse_reference))
        .route("/v2/parse/upload-url", post(firecrawl_v2_parse_upload_url))
        .route("/v2/parse/upload/:id", put(firecrawl_v2_parse_upload))
        .route("/v2/crawl", post(firecrawl_v2_crawl))
        .route("/v2/crawl/active", get(firecrawl_v2_active_crawls))
        .route("/v2/crawl/ongoing", get(firecrawl_v2_active_crawls))
        .route("/v2/crawl/:id/errors", get(firecrawl_v2_job_errors))
        .route(
            "/v2/crawl/:id",
            get(firecrawl_v2_crawl_status).delete(firecrawl_v2_cancel_crawl),
        )
        .route("/v2/batch/scrape", post(firecrawl_v2_batch_scrape))
        .route("/v2/batch/scrape/:id/errors", get(firecrawl_v2_job_errors))
        .route(
            "/v2/batch/scrape/:id",
            get(firecrawl_v2_batch_scrape_status).delete(firecrawl_v2_cancel_crawl),
        )
        .route("/v2/map", post(firecrawl_v2_map))
        .route("/v2/extract", post(firecrawl_v2_extract))
        .route("/v2/search", post(firecrawl_v2_search))
        .route(
            "/v2/browser",
            post(firecrawl_v2_browser_create).get(firecrawl_v2_browser_list),
        )
        .route(
            "/v2/interact",
            post(firecrawl_v2_browser_create).get(firecrawl_v2_browser_list),
        )
        .route(
            "/v2/browser/:id/execute",
            post(firecrawl_v2_browser_execute),
        )
        .route(
            "/v2/interact/:id/execute",
            post(firecrawl_v2_browser_execute),
        )
        .route("/v2/browser/:id/replay", get(firecrawl_v2_browser_replay))
        .route("/v2/interact/:id/replay", get(firecrawl_v2_browser_replay))
        .route(
            "/v2/browser/:id/replay/:page_id",
            get(firecrawl_v2_browser_replay_page),
        )
        .route(
            "/v2/interact/:id/replay/:page_id",
            get(firecrawl_v2_browser_replay_page),
        )
        .route(
            "/v2/browser/:id",
            axum::routing::delete(firecrawl_v2_browser_delete),
        )
        .route(
            "/v2/interact/:id",
            axum::routing::delete(firecrawl_v2_browser_delete),
        )
        .route(
            "/v2/scrape/:id/interact",
            post(firecrawl_v2_scrape_interact).delete(firecrawl_v2_scrape_interact_delete),
        )
        .route("/v2/agent", post(create_agent))
        .route("/v2/agent/:id", get(get_agent).delete(cancel_agent))
        .route("/v2/monitor", post(create_monitor).get(list_monitors))
        .route(
            "/v2/monitor/:id",
            get(get_monitor)
                .patch(update_monitor)
                .delete(delete_monitor),
        )
        .route("/v2/monitor/:id/run", post(run_monitor))
        .route("/v2/monitor/:id/checks", get(monitor_checks))
        .route("/v2/monitor/:id/checks/:check_id", get(get_monitor_check))
        .layer(TraceLayer::new_for_http())
        .layer(DefaultBodyLimit::max(70 * 1024 * 1024))
        .with_state(Arc::new(state))
}

async fn create_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<AgentCreateRequest>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    if request.prompt.trim().is_empty() || request.prompt.len() > 20_000 {
        return Err(ApiError::InvalidRequest(
            "prompt must contain 1 to 20000 bytes".to_string(),
        ));
    }
    if request.urls.len() > 100 {
        return Err(ApiError::InvalidRequest(
            "urls cannot contain more than 100 source URLs".to_string(),
        ));
    }
    if !(1..=100).contains(&request.max_credits) || request.urls.len() > request.max_credits {
        return Err(ApiError::InvalidRequest(
            "maxCredits must be 1 to 100 and cover all source URLs".to_string(),
        ));
    }
    for url in &request.urls {
        web_extract::normalize_url(url)?;
    }
    let id = state
        .workflows
        .create_agent(&browser_owner(&headers), request)
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"success": true, "id": id})),
    )
        .into_response())
}

async fn get_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    Ok(Json(state.workflows.agent(&browser_owner(&headers), id).await?).into_response())
}

async fn cancel_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    Ok(Json(
        state
            .workflows
            .cancel_agent(&browser_owner(&headers), id)
            .await?,
    )
    .into_response())
}

async fn create_monitor(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<MonitorCreateRequest>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    if request.name.trim().is_empty() || request.name.len() > 200 {
        return Err(ApiError::InvalidRequest(
            "name must contain 1 to 200 bytes".to_string(),
        ));
    }
    web_extract::normalize_url(&request.url)?;
    if !(60..=31_536_000).contains(&request.schedule_seconds) {
        return Err(ApiError::InvalidRequest(
            "scheduleSeconds must be between 60 and 31536000".to_string(),
        ));
    }
    if let Some(webhook) = &request.webhook {
        webhook::validate(webhook).map_err(|error| ApiError::InvalidRequest(error.to_string()))?;
    }
    let monitor = state
        .workflows
        .create_monitor(&browser_owner(&headers), request)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"success": true, "data": monitor})),
    )
        .into_response())
}

async fn list_monitors(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let monitors = state.workflows.monitors(&browser_owner(&headers)).await?;
    Ok(Json(json!({"success": true, "data": monitors})).into_response())
}

async fn get_monitor(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let monitor = state
        .workflows
        .monitor(&browser_owner(&headers), id)
        .await?;
    Ok(Json(json!({"success": true, "data": monitor})).into_response())
}

async fn update_monitor(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Json(request): Json<MonitorUpdateRequest>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    if request
        .name
        .as_ref()
        .is_some_and(|name| name.trim().is_empty() || name.len() > 200)
    {
        return Err(ApiError::InvalidRequest(
            "name must contain 1 to 200 bytes".to_string(),
        ));
    }
    if let Some(url) = &request.url {
        web_extract::normalize_url(url)?;
    }
    if request
        .schedule_seconds
        .is_some_and(|seconds| !(60..=31_536_000).contains(&seconds))
    {
        return Err(ApiError::InvalidRequest(
            "scheduleSeconds must be between 60 and 31536000".to_string(),
        ));
    }
    if let Some(webhook) = &request.webhook {
        webhook::validate(webhook).map_err(|error| ApiError::InvalidRequest(error.to_string()))?;
    }
    let monitor = state
        .workflows
        .update_monitor(&browser_owner(&headers), id, request)
        .await?;
    Ok(Json(json!({"success": true, "data": monitor})).into_response())
}

async fn delete_monitor(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    state
        .workflows
        .delete_monitor(&browser_owner(&headers), id)
        .await?;
    Ok(Json(json!({"success": true, "id": id})).into_response())
}

async fn run_monitor(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let check_id = state
        .workflows
        .run_monitor(&browser_owner(&headers), id)
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"success": true, "id": check_id, "monitorId": id})),
    )
        .into_response())
}

async fn monitor_checks(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let checks = state.workflows.checks(&browser_owner(&headers), id).await?;
    Ok(Json(json!({"success": true, "data": checks})).into_response())
}

async fn get_monitor_check(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let check = state
        .workflows
        .check(&browser_owner(&headers), id, check_id)
        .await?;
    Ok(Json(json!({"success": true, "data": check})).into_response())
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
            skip_tls_verification: false,
            headers: HashMap::new(),
            proxy: None,
            screenshot: None,
            content: None,
            actions: vec![],
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
    payload: Result<Json<FirecrawlV2ScrapeRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let Json(request) = firecrawl_json(payload)?;
    validate_browser_actions(&request.actions, request.timeout, request.wait_for_ms)?;
    validate_firecrawl_scrape_options(
        request.only_main_content,
        request.remove_base64_images,
        request.fast_mode,
        request.block_ads,
        request.store_in_cache,
        request.max_age,
        request.mobile,
    )?;
    let requested_formats = request.formats;
    let proxy = firecrawl_proxy(request.proxy.as_deref())?;
    validate_firecrawl_enrichment_formats(&requested_formats)?;
    let screenshot = firecrawl_screenshot_options(&requested_formats)?;
    let content = ContentOptions {
        only_main_content: request.only_main_content.unwrap_or(true),
        only_clean_content: request.only_clean_content,
        include_tags: request.include_tags,
        exclude_tags: request.exclude_tags,
    };
    let mut fetch_formats = firecrawl_format_names(&requested_formats);
    let requested_raw_html = fetch_formats.iter().any(|format| format == "rawHtml");
    if requested_formats
        .iter()
        .any(|format| matches!(format.name(), "images" | "attributes"))
        && !requested_raw_html
    {
        fetch_formats.push("rawHtml".to_string());
    }
    let source_url = request.url.clone();
    let response = web_extract::scrape_with_cache(
        &state.client,
        &state.cache,
        WebExtractScrapeRequest {
            url: request.url,
            formats: fetch_formats,
            location: request.location,
            timeout_seconds: request.timeout.div_ceil(1_000).max(1),
            wait_for_ms: request.wait_for_ms,
            use_browser: "auto".to_string(),
            skip_tls_verification: request.skip_tls_verification.unwrap_or(false),
            headers: request.headers,
            proxy,
            screenshot,
            content: Some(content),
            actions: request.actions,
        },
    )
    .await?;
    if response.metadata.rendered {
        state.scrape_handoffs.lock().await.insert(
            response.request_id.clone(),
            ScrapeHandoff {
                url: response.final_url.clone(),
                storage_state: response.browser_state.clone(),
                created_at: chrono::Utc::now(),
                session_id: None,
            },
        );
    }
    let mut document = firecrawl_document(response);
    enrich_firecrawl_document(
        &state,
        &source_url,
        &requested_formats,
        requested_raw_html,
        &mut document,
    )
    .await?;
    Ok(Json(json!({ "success": true, "data": document })).into_response())
}

async fn firecrawl_v2_parse(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let mut file = None;
    let mut filename = None;
    let mut upload_ref = None;
    let mut options = FirecrawlV2ParseOptions::default();

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
            Some("uploadRef") => {
                upload_ref = Some(field.text().await.map_err(|error| {
                    ApiError::InvalidRequest(format!("Could not read uploadRef: {error}"))
                })?);
            }
            _ => {}
        }
    }

    if let Some(upload_ref) = upload_ref {
        let upload = take_parse_upload(&state, &upload_ref).await?;
        filename = Some(upload.filename);
        file = upload.data;
    }

    parse_document_response(
        &state.client,
        filename.ok_or_else(|| ApiError::InvalidRequest("Missing file field".to_string()))?,
        file.ok_or_else(|| ApiError::InvalidRequest("Missing file field".to_string()))?,
        options,
    )
    .await
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ParseUploadInitRequest {
    filename: String,
    #[serde(rename = "contentType", default = "default_upload_content_type")]
    content_type: String,
    #[serde(rename = "declaredSizeBytes")]
    declared_size_bytes: Option<usize>,
}

fn default_upload_content_type() -> String {
    "application/octet-stream".to_string()
}

#[derive(serde::Deserialize)]
struct ParseUploadQuery {
    #[serde(rename = "uploadRef")]
    upload_ref: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ParseReferenceRequest {
    #[serde(rename = "uploadRef")]
    upload_ref: String,
    #[serde(flatten)]
    options: FirecrawlV2ParseOptions,
}

async fn firecrawl_v2_parse_upload_url(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<ParseUploadInitRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let Json(request) = firecrawl_json(payload)?;
    if request.filename.len() > 512 || request.filename.contains(['/', '\\']) {
        return Err(ApiError::InvalidRequest(
            "Invalid upload filename".to_string(),
        ));
    }
    if request
        .declared_size_bytes
        .is_some_and(|size| size == 0 || size > 50 * 1024 * 1024)
    {
        return Err(ApiError::InvalidRequest(
            "declaredSizeBytes must be between 1 and 52428800".to_string(),
        ));
    }
    let extension = std::path::Path::new(&request.filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "html" | "htm" | "xhtml" | "pdf" | "doc" | "docx" | "odt" | "rtf" | "xls" | "xlsx"
    ) {
        return Err(ApiError::InvalidRequest(
            "Unsupported upload type".to_string(),
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now() + chrono::Duration::minutes(15);
    state.parse_uploads.lock().await.insert(
        id.clone(),
        ParseUpload {
            filename: request.filename,
            expires_at,
            data: None,
        },
    );
    Ok(Json(json!({
        "success": true,
        "data": {
            "uploadUrl": format!("/v2/parse/upload/{id}?uploadRef={id}"),
            "uploadRef": id,
            "method": "PUT",
            "headers": { "Content-Type": request.content_type },
            "expiresAt": expires_at,
            "maxSizeBytes": 50 * 1024 * 1024,
        }
    }))
    .into_response())
}

async fn firecrawl_v2_parse_upload(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ParseUploadQuery>,
    body: Bytes,
) -> Result<Response, ApiError> {
    if id != query.upload_ref || body.is_empty() || body.len() > 50 * 1024 * 1024 {
        return Err(ApiError::InvalidRequest("Invalid parse upload".to_string()));
    }
    let mut uploads = state.parse_uploads.lock().await;
    let upload = uploads.get_mut(&id).ok_or(ApiError::NotFound)?;
    if upload.expires_at <= chrono::Utc::now() {
        uploads.remove(&id);
        return Err(ApiError::InvalidRequest(
            "uploadRef has expired".to_string(),
        ));
    }
    upload.data = Some(body);
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn firecrawl_v2_parse_reference(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<ParseReferenceRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let Json(request) = firecrawl_json(payload)?;
    let upload = take_parse_upload(&state, &request.upload_ref).await?;
    parse_document_response(
        &state.client,
        upload.filename,
        upload
            .data
            .ok_or_else(|| ApiError::InvalidRequest("Upload is incomplete".to_string()))?,
        request.options,
    )
    .await
}

async fn take_parse_upload(state: &AppState, upload_ref: &str) -> Result<ParseUpload, ApiError> {
    let mut uploads = state.parse_uploads.lock().await;
    let upload = uploads.get(upload_ref).cloned().ok_or(ApiError::NotFound)?;
    if upload.expires_at <= chrono::Utc::now() {
        uploads.remove(upload_ref);
        return Err(ApiError::InvalidRequest(
            "uploadRef has expired".to_string(),
        ));
    }
    if upload.data.is_none() {
        return Err(ApiError::InvalidRequest("Upload is incomplete".to_string()));
    }
    uploads.remove(upload_ref);
    Ok(upload)
}

async fn firecrawl_v2_parse_base64(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<FirecrawlV2Base64ParseRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let Json(request) = firecrawl_json(payload)?;
    let bytes = decode_base64_document(&request.base64)?;
    parse_document_response(
        &state.client,
        request.filename,
        bytes.into(),
        request.options,
    )
    .await
}

fn decode_base64_document(value: &str) -> Result<Vec<u8>, ApiError> {
    let trimmed = value.trim();
    let encoded = if trimmed.starts_with("data:") {
        trimmed
            .split_once(",")
            .map(|(_, data)| data)
            .ok_or_else(|| ApiError::InvalidRequest("Invalid document data URL".to_string()))?
    } else {
        trimmed
    };
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| ApiError::InvalidRequest(format!("Invalid document base64 data: {error}")))
}

async fn parse_document_response(
    client: &reqwest::Client,
    filename: String,
    bytes: axum::body::Bytes,
    options: FirecrawlV2ParseOptions,
) -> Result<Response, ApiError> {
    if bytes.len() > 50 * 1024 * 1024 {
        return Err(ApiError::InvalidRequest(
            "Document file must not exceed 50 MB".to_string(),
        ));
    }
    let extension = std::path::Path::new(&filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "html" | "htm" | "xhtml" | "pdf" | "doc" | "docx" | "odt" | "rtf" | "xls" | "xlsx"
    ) {
        return Err(ApiError::InvalidRequest(
            "Unsupported file type for /v2/parse".to_string(),
        ));
    }
    let is_pdf = extension == "pdf";
    if is_pdf && !bytes.starts_with(b"%PDF-") {
        return Err(ApiError::InvalidRequest(
            "Uploaded file is not a PDF".to_string(),
        ));
    }
    if options.formats.iter().any(|format| {
        !matches!(
            format.name(),
            "markdown" | "html" | "rawHtml" | "summary" | "json"
        )
    }) {
        return Err(ApiError::InvalidRequest(
            "Parse supports markdown, html, rawHtml, summary, and json formats".to_string(),
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
        if !matches!(mode, "fast" | "auto" | "ocr") {
            return Err(ApiError::InvalidRequest(
                "PDF mode must be fast, auto, or ocr".to_string(),
            ));
        }
    }
    let max_pages = parser.and_then(|parser| parser.max_pages);
    if matches!(max_pages, Some(0)) || max_pages.is_some_and(|value| value > 10_000) {
        return Err(ApiError::InvalidRequest(
            "PDF maxPages must be between 1 and 10000".to_string(),
        ));
    }
    let mode = parser
        .and_then(|parser| parser.mode.clone())
        .unwrap_or_else(|| "auto".to_string());
    let format_names = firecrawl_format_names(&options.formats);
    if is_pdf && mode != "ocr" && format_names == ["markdown"] {
        let local_bytes = bytes.clone();
        let parsed =
            tokio::task::spawn_blocking(move || crate::pdf::parse(&local_bytes, max_pages))
                .await
                .map_err(|error| ApiError::Internal(format!("PDF parser failed: {error}")))?
                .map_err(ApiError::InvalidRequest)?;
        if mode == "fast" || !parsed.markdown.trim().is_empty() {
            return Ok(Json(json!({
                "success": true,
                "data": {
                    "markdown": parsed.markdown,
                    "metadata": {
                        "numPages": parsed.num_pages,
                        "totalPages": parsed.total_pages,
                        "ocrPages": 0,
                        "sourceFile": filename,
                        "sourceURL": filename,
                    }
                }
            }))
            .into_response());
        }
    }
    let engine_url =
        std::env::var("BEE_ENGINE_URL").unwrap_or_else(|_| "http://127.0.0.1:8020".to_string());
    let response = client
        .post(format!("{}/parse", engine_url.trim_end_matches('/')))
        .json(&json!({
            "filename": filename,
            "base64": base64::engine::general_purpose::STANDARD.encode(&bytes),
            "formats": format_names,
            "mode": mode,
            "maxPages": max_pages,
        }))
        .timeout(std::time::Duration::from_millis(options.timeout.max(1_000)))
        .send()
        .await
        .map_err(|error| ApiError::Internal(format!("Document parser unavailable: {error}")))?;
    if !response.status().is_success() {
        return Err(ApiError::InvalidRequest(format!(
            "Document parser returned HTTP {}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        )));
    }
    let parsed: Value = response.json().await.map_err(|error| {
        ApiError::Internal(format!("Invalid document parser response: {error}"))
    })?;
    let mut data = parsed["data"].clone();
    data["metadata"] = parsed["metadata"].clone();
    data["metadata"]["sourceFile"] = json!(filename);
    data["metadata"]["sourceURL"] = json!(filename);
    Ok(Json(json!({ "success": true, "data": data })).into_response())
}

async fn firecrawl_v2_map(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<FirecrawlV2MapRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let Json(request) = firecrawl_json(payload)?;
    let response = web_extract::map_site(&state.client, request.into()).await?;
    let links = firecrawl_map_links(response.links);
    Ok(Json(json!({ "success": true, "id": response.request_id, "links": links })).into_response())
}

async fn firecrawl_v2_crawl(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<FirecrawlV2CrawlRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let Json(request) = firecrawl_json(payload)?;
    let idempotency_key = firecrawl_idempotency_key(&headers)?;
    validate_firecrawl_crawl_defaults(&request)?;
    let delay_ms = firecrawl_delay_ms(request.delay)?;
    let max_concurrency = firecrawl_max_concurrency(request.max_concurrency)?;
    if let Some(config) = request.webhook.as_ref() {
        webhook::validate(config).map_err(|error| ApiError::InvalidRequest(error.to_string()))?;
    }
    let scrape = request.scrape_options.unwrap_or_default();
    validate_browser_actions(&scrape.actions, scrape.timeout, scrape.wait_for_ms)?;
    let proxy = firecrawl_proxy(scrape.proxy.as_deref())?;
    validate_firecrawl_scrape_options(
        scrape.only_main_content,
        scrape.remove_base64_images,
        scrape.fast_mode,
        scrape.block_ads,
        scrape.store_in_cache,
        scrape.max_age,
        scrape.mobile,
    )?;
    if scrape
        .formats
        .iter()
        .any(|format| !format.name().eq_ignore_ascii_case("markdown"))
    {
        return Err(ApiError::InvalidRequest(
            "BeeCrawl crawl currently supports only the markdown scrape format".to_string(),
        ));
    }
    let response = state
        .crawls
        .enqueue(CrawlRequest {
            url: request.url,
            idempotency_key,
            webhook: request.webhook,
            proxy,
            limit: request.limit,
            max_depth: request.max_discovery_depth,
            include_paths: request.include_paths,
            exclude_paths: request.exclude_paths,
            regex_on_full_url: request.regex_on_full_url.unwrap_or(false),
            include_subdomains: request.allow_subdomains,
            allow_external_links: request.allow_external_links.unwrap_or(false),
            crawl_entire_domain: request.crawl_entire_domain.unwrap_or(false),
            sitemap: request.sitemap,
            delay_ms,
            max_concurrency,
            deduplicate_similar_urls: request.deduplicate_similar_urls.unwrap_or(true),
            ignore_query_parameters: request.ignore_query_parameters,
            ignore_robots_txt: request.ignore_robots_txt.unwrap_or(false),
            robots_user_agent: request.robots_user_agent,
            timeout_seconds: scrape.timeout.div_ceil(1_000).max(1),
            wait_for_ms: scrape.wait_for_ms,
            use_browser: "auto".to_string(),
            skip_tls_verification: scrape.skip_tls_verification.unwrap_or(false),
            max_retries: 2,
            actions: scrape.actions,
        })
        .await?;
    let status_url = format!("/v2/crawl/{}", response.id);
    Ok(Json(json!({ "success": true, "id": response.id, "url": status_url })).into_response())
}

async fn firecrawl_v2_batch_scrape(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<FirecrawlV2BatchScrapeRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let Json(request) = firecrawl_json(payload)?;
    let max_concurrency = firecrawl_max_concurrency(request.max_concurrency)?;
    if let Some(config) = request.webhook.as_ref() {
        webhook::validate(config).map_err(|error| ApiError::InvalidRequest(error.to_string()))?;
    }
    let scrape = request.scrape_options;
    validate_browser_actions(&scrape.actions, scrape.timeout, scrape.wait_for_ms)?;
    let proxy = firecrawl_proxy(scrape.proxy.as_deref())?;
    validate_firecrawl_scrape_options(
        scrape.only_main_content,
        scrape.remove_base64_images,
        scrape.fast_mode,
        scrape.block_ads,
        scrape.store_in_cache,
        scrape.max_age,
        scrape.mobile,
    )?;
    if scrape
        .formats
        .iter()
        .any(|format| !format.name().eq_ignore_ascii_case("markdown"))
    {
        return Err(ApiError::InvalidRequest(
            "BeeCrawl batch scrape currently supports only the markdown format".to_string(),
        ));
    }
    let response = state
        .crawls
        .enqueue_batch(BatchScrapeRequest {
            urls: request.urls,
            max_concurrency,
            webhook: request.webhook,
            proxy,
            timeout_seconds: scrape.timeout.div_ceil(1_000).max(1),
            wait_for_ms: scrape.wait_for_ms,
            use_browser: "auto".to_string(),
            skip_tls_verification: scrape.skip_tls_verification.unwrap_or(false),
            max_retries: 2,
            actions: scrape.actions,
        })
        .await?;
    let status_url = format!("/v2/batch/scrape/{}", response.id);
    Ok(Json(json!({
        "success": true,
        "id": response.id,
        "url": status_url,
    }))
    .into_response())
}

async fn firecrawl_v2_crawl_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Query(query): Query<CrawlStatusQuery>,
    websocket: Option<WebSocketUpgrade>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    if let Some(websocket) = websocket {
        state
            .crawls
            .get(&id, CrawlStatusQuery::default())
            .await?
            .ok_or(ApiError::NotFound)?;
        return Ok(websocket
            .on_upgrade(move |socket| watch_firecrawl_job(socket, state, id, "crawl"))
            .into_response());
    }
    let response = state
        .crawls
        .get(&id, query)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(firecrawl_crawl_status(response, "crawl")).into_response())
}

async fn firecrawl_v2_batch_scrape_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Query(query): Query<CrawlStatusQuery>,
    websocket: Option<WebSocketUpgrade>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    if let Some(websocket) = websocket {
        state
            .crawls
            .get(&id, CrawlStatusQuery::default())
            .await?
            .ok_or(ApiError::NotFound)?;
        return Ok(websocket
            .on_upgrade(move |socket| watch_firecrawl_job(socket, state, id, "batch/scrape"))
            .into_response());
    }
    let response = state
        .crawls
        .get(&id, query)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(firecrawl_crawl_status(response, "batch/scrape")).into_response())
}

async fn watch_firecrawl_job(
    mut socket: WebSocket,
    state: Arc<AppState>,
    id: String,
    resource: &'static str,
) {
    let mut sent = std::collections::HashSet::new();
    let mut first = true;
    loop {
        let status = match firecrawl_complete_status(&state.crawls, &id).await {
            Ok(Some(status)) => status,
            Ok(None) => {
                let _ = websocket_send(
                    &mut socket,
                    json!({
                        "type": "error",
                        "error": "Job not found"
                    }),
                )
                .await;
                break;
            }
            Err(error) => {
                let _ = websocket_send(
                    &mut socket,
                    json!({
                        "type": "error",
                        "error": error.to_string()
                    }),
                )
                .await;
                break;
            }
        };
        let terminal = matches!(status.status.as_str(), "completed" | "cancelled");
        if first {
            let catchup = firecrawl_crawl_status(status.clone(), resource);
            if websocket_send(&mut socket, json!({ "type": "catchup", "data": catchup }))
                .await
                .is_err()
            {
                break;
            }
            sent.extend(status.data.iter().map(|page| page.request_id.clone()));
            first = false;
        } else {
            for page in status.data {
                if sent.insert(page.request_id.clone())
                    && websocket_send(
                        &mut socket,
                        json!({ "type": "document", "data": firecrawl_document(page) }),
                    )
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
        if terminal {
            let _ = websocket_send(&mut socket, json!({ "type": "done" })).await;
            let _ = socket.close().await;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

async fn websocket_send(socket: &mut WebSocket, payload: serde_json::Value) -> Result<(), ()> {
    socket
        .send(Message::Text(payload.to_string()))
        .await
        .map_err(|_| ())
}

async fn firecrawl_complete_status(
    store: &CrawlStore,
    id: &str,
) -> Result<Option<CrawlStatusResponse>, CrawlStoreError> {
    let Some(mut status) = store
        .get(
            id,
            CrawlStatusQuery {
                offset: 0,
                limit: 100,
            },
        )
        .await?
    else {
        return Ok(None);
    };
    let mut offset = status.pagination.next;
    while let Some(next) = offset {
        let Some(page) = store
            .get(
                id,
                CrawlStatusQuery {
                    offset: next,
                    limit: 100,
                },
            )
            .await?
        else {
            break;
        };
        status.data.extend(page.data);
        status.errors.extend(page.errors);
        offset = page.pagination.next;
    }
    status.pagination.next = None;
    status.pagination.offset = 0;
    status.pagination.limit = status.data.len() + status.errors.len();
    Ok(Some(status))
}

async fn firecrawl_v2_job_errors(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let response = state.crawls.errors(&id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(response).into_response())
}

async fn firecrawl_v2_active_crawls(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    Ok(Json(state.crawls.active().await?).into_response())
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
    payload: Result<Json<FirecrawlV2ExtractRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let Json(request) = firecrawl_json(payload)?;
    if request.urls.is_empty() {
        return Err(ApiError::InvalidRequest(
            "Firecrawl v2 extract requires at least one URL".to_string(),
        ));
    }
    if request.enable_web_search {
        return Err(ApiError::InvalidRequest(
            "BeeCrawl extract does not support enableWebSearch".to_string(),
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
                    skip_tls_verification: false,
                    headers: HashMap::new(),
                    proxy: None,
                    screenshot: None,
                    content: None,
                    actions: vec![],
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
    payload: Result<Json<FirecrawlV2SearchRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let Json(request) = firecrawl_json(payload)?;
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
    if request
        .timeout
        .is_some_and(|timeout| !(1_000..=300_000).contains(&timeout))
    {
        return Err(ApiError::InvalidRequest(
            "Firecrawl v2 search timeout must be between 1000 and 300000 milliseconds".to_string(),
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

    if !request.include_domains.is_empty() && !request.exclude_domains.is_empty() {
        return Err(ApiError::InvalidRequest(
            "includeDomains and excludeDomains cannot both be specified".to_string(),
        ));
    }
    if request
        .categories
        .iter()
        .any(|category| !matches!(category.name(), "github" | "research" | "pdf"))
    {
        return Err(ApiError::InvalidRequest(
            "categories must be github, research, or pdf".to_string(),
        ));
    }
    let country = request.country.clone().unwrap_or_else(|| "us".to_string());
    let news = if wants_news {
        search::search_news(
            &state.client,
            &request.query,
            request.limit,
            &request.lang,
            &country,
            request.tbs.as_deref(),
        )
        .await
    } else {
        vec![]
    };
    let images = if wants_images {
        search::search_images(&state.client, &request.query, request.limit).await
    } else {
        vec![]
    };

    if !wants_web {
        let mut data = serde_json::Map::new();
        if wants_news {
            data.insert("news".to_string(), json!(news));
        }
        if wants_images {
            data.insert("images".to_string(), json!(images));
        }
        return Ok(Json(json!({ "success": true, "data": data })).into_response());
    }

    if let Some(options) = &request.scrape_options {
        validate_browser_actions(&options.actions, options.timeout, options.wait_for_ms)?;
        validate_firecrawl_scrape_options(
            options.only_main_content,
            options.remove_base64_images,
            options.fast_mode,
            options.block_ads,
            options.store_in_cache,
            options.max_age,
            options.mobile,
        )?;
    }
    let search_proxy = request
        .scrape_options
        .as_ref()
        .map(|options| firecrawl_proxy(options.proxy.as_deref()))
        .transpose()?
        .flatten();
    let categories = request
        .categories
        .iter()
        .map(|category| crate::models::SearchCategory {
            name: category.name().to_string(),
            sites: category.sites().to_vec(),
        })
        .collect();

    let response = search::search(
        &state.client,
        SearchRequest {
            query: request.query,
            limit: request.limit,
            lang: request.lang,
            country,
            categories,
            include_domains: request.include_domains,
            exclude_domains: request.exclude_domains,
            tbs: request.tbs,
            location: request.location,
            filter: request.filter,
            async_scraping: request.async_scraping,
            highlights: request.highlights,
            scrape_options: request.scrape_options.map(|options| SearchScrapeOptions {
                formats: firecrawl_format_names(&options.formats),
                timeout_seconds: options.timeout.div_ceil(1_000).max(1),
                wait_for_ms: options.wait_for_ms,
                use_browser: "auto".to_string(),
                skip_tls_verification: options.skip_tls_verification.unwrap_or(false),
                headers: options.headers,
                proxy: search_proxy,
                content: Some(ContentOptions {
                    only_main_content: options.only_main_content.unwrap_or(true),
                    only_clean_content: options.only_clean_content,
                    include_tags: options.include_tags,
                    exclude_tags: options.exclude_tags,
                }),
                actions: options.actions,
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
        data.insert("news".to_string(), json!(news));
    }
    if wants_images {
        data.insert("images".to_string(), json!(images));
    }
    Ok(Json(json!({ "success": true, "data": data })).into_response())
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BrowserCreateRequest {
    #[serde(default = "default_browser_ttl")]
    ttl: u64,
    #[serde(default = "default_browser_activity_ttl")]
    activity_ttl: u64,
    #[serde(default = "browser_default_true")]
    stream_web_view: bool,
    #[serde(default = "browser_default_true")]
    record_session: bool,
    #[serde(default)]
    initial_url: Option<String>,
    #[serde(default)]
    storage_state: Option<Value>,
}

fn default_browser_ttl() -> u64 {
    600
}

fn browser_default_true() -> bool {
    true
}

fn default_browser_activity_ttl() -> u64 {
    300
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct BrowserExecuteRequest {
    code: String,
    #[serde(default = "default_browser_language")]
    language: String,
    #[serde(default = "default_browser_execute_timeout")]
    timeout: u64,
    #[serde(default)]
    origin: Option<String>,
}

fn default_browser_language() -> String {
    "node".to_string()
}

fn default_browser_execute_timeout() -> u64 {
    30
}

async fn firecrawl_v2_browser_create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<BrowserCreateRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let Json(request) = firecrawl_json(payload)?;
    Ok(
        create_browser_session(&state, browser_owner(&headers), request)
            .await?
            .0,
    )
}

async fn create_browser_session(
    state: &AppState,
    owner: String,
    request: BrowserCreateRequest,
) -> Result<(Response, String), ApiError> {
    if !(30..=3_600).contains(&request.ttl) || !(10..=3_600).contains(&request.activity_ttl) {
        return Err(ApiError::InvalidRequest(
            "ttl must be 30..3600 and activityTtl must be 10..3600 seconds".to_string(),
        ));
    }
    if request
        .initial_url
        .as_ref()
        .is_some_and(|url| web_extract::normalize_url(url).is_err())
    {
        return Err(ApiError::InvalidRequest("Invalid initialUrl".to_string()));
    }
    let response = state
        .client
        .post(browser_engine_url("/sessions"))
        .json(&json!({
            "ttl": request.ttl,
            "activityTtl": request.activity_ttl,
            "initialUrl": request.initial_url,
            "storageState": request.storage_state,
            "record": request.record_session,
        }))
        .send()
        .await
        .map_err(|error| ApiError::Internal(format!("Browser service unavailable: {error}")))?;
    if !response.status().is_success() {
        return Err(ApiError::InvalidRequest(format!(
            "Browser service returned HTTP {}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        )));
    }
    let session: Value = response
        .json()
        .await
        .map_err(|error| ApiError::Internal(format!("Invalid browser response: {error}")))?;
    let id = session["id"]
        .as_str()
        .ok_or_else(|| ApiError::Internal("Browser response has no session ID".to_string()))?
        .to_string();
    state.browser_owners.lock().await.insert(id.clone(), owner);
    let response = Json(json!({
        "success": true,
        "id": id,
        "cdpUrl": Value::Null,
        "liveViewUrl": Value::Null,
        "interactiveLiveViewUrl": Value::Null,
        "expiresAt": session["expiresAt"],
    }))
    .into_response();
    Ok((response, id))
}

async fn firecrawl_v2_browser_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let owner = browser_owner(&headers);
    let response = state
        .client
        .get(browser_engine_url("/sessions"))
        .send()
        .await
        .map_err(|error| ApiError::Internal(format!("Browser service unavailable: {error}")))?;
    let payload: Value = response
        .json()
        .await
        .map_err(|error| ApiError::Internal(format!("Invalid browser response: {error}")))?;
    let owners = state.browser_owners.lock().await;
    let sessions = payload["sessions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|session| {
            session["id"]
                .as_str()
                .and_then(|id| owners.get(id))
                .is_some_and(|session_owner| session_owner == &owner)
        })
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(json!({ "success": true, "sessions": sessions })).into_response())
}

async fn firecrawl_v2_browser_execute(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    payload: Result<Json<BrowserExecuteRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    require_browser_owner(&state, &headers, &id).await?;
    let Json(request) = firecrawl_json(payload)?;
    if request.code.is_empty()
        || request.code.len() > 100_000
        || !(1..=300).contains(&request.timeout)
    {
        return Err(ApiError::InvalidRequest(
            "code must be 1..100000 bytes and timeout 1..300 seconds".to_string(),
        ));
    }
    proxy_browser_json(
        &state,
        reqwest::Method::POST,
        &format!("/sessions/{id}/execute"),
        Some(serde_json::to_value(request).expect("browser execution serializes")),
    )
    .await
}

async fn firecrawl_v2_browser_replay(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    require_browser_owner(&state, &headers, &id).await?;
    proxy_browser_json(
        &state,
        reqwest::Method::GET,
        &format!("/sessions/{id}/replay"),
        None,
    )
    .await
}

async fn firecrawl_v2_browser_replay_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((id, page_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    require_browser_owner(&state, &headers, &id).await?;
    proxy_browser_json(
        &state,
        reqwest::Method::GET,
        &format!("/sessions/{id}/replay/{page_id}"),
        None,
    )
    .await
}

async fn firecrawl_v2_browser_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    require_browser_owner(&state, &headers, &id).await?;
    let response = proxy_browser_json(
        &state,
        reqwest::Method::DELETE,
        &format!("/sessions/{id}"),
        None,
    )
    .await?;
    state.browser_owners.lock().await.remove(&id);
    Ok(response)
}

async fn firecrawl_v2_scrape_interact(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let handoff = state
        .scrape_handoffs
        .lock()
        .await
        .get(&id)
        .cloned()
        .ok_or(ApiError::NotFound)?;
    if chrono::Utc::now() - handoff.created_at > chrono::Duration::hours(4) {
        return Err(ApiError::NotFound);
    }
    let (response, session_id) = create_browser_session(
        &state,
        browser_owner(&headers),
        BrowserCreateRequest {
            ttl: 600,
            activity_ttl: 300,
            stream_web_view: true,
            record_session: true,
            initial_url: Some(handoff.url),
            storage_state: handoff.storage_state,
        },
    )
    .await?;
    if let Some(handoff) = state.scrape_handoffs.lock().await.get_mut(&id) {
        handoff.session_id = Some(session_id);
    }
    Ok(response)
}

async fn firecrawl_v2_scrape_interact_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require_auth(&headers)?;
    let session_id = state
        .scrape_handoffs
        .lock()
        .await
        .get(&id)
        .and_then(|handoff| handoff.session_id.clone())
        .ok_or(ApiError::NotFound)?;
    require_browser_owner(&state, &headers, &session_id).await?;
    let response = proxy_browser_json(
        &state,
        reqwest::Method::DELETE,
        &format!("/sessions/{session_id}"),
        None,
    )
    .await?;
    state.browser_owners.lock().await.remove(&session_id);
    Ok(response)
}

async fn proxy_browser_json(
    state: &AppState,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Response, ApiError> {
    let mut request = state.client.request(method, browser_engine_url(path));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| ApiError::Internal(format!("Browser service unavailable: {error}")))?;
    let status = response.status();
    let mut payload: Value = response.json().await.unwrap_or_else(|_| json!({}));
    if !status.is_success() {
        return Err(if status == reqwest::StatusCode::NOT_FOUND {
            ApiError::NotFound
        } else {
            ApiError::InvalidRequest(
                payload["detail"]
                    .as_str()
                    .unwrap_or("Browser request failed")
                    .to_string(),
            )
        });
    }
    if let Some(object) = payload.as_object_mut() {
        object.insert("success".to_string(), json!(true));
    }
    Ok(Json(payload).into_response())
}

fn browser_engine_url(path: &str) -> String {
    format!(
        "{}{}",
        std::env::var("BEE_ENGINE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8020".to_string())
            .trim_end_matches('/'),
        path
    )
}

fn browser_owner(headers: &HeaderMap) -> String {
    let credential = headers
        .get("authorization")
        .or_else(|| headers.get("x-api-key"))
        .and_then(|value| value.to_str().ok())
        .unwrap_or("anonymous");
    format!("{:x}", Sha256::digest(credential.as_bytes()))
}

async fn require_browser_owner(
    state: &AppState,
    headers: &HeaderMap,
    id: &str,
) -> Result<(), ApiError> {
    let owner = browser_owner(headers);
    match state.browser_owners.lock().await.get(id) {
        Some(session_owner) if session_owner == &owner => Ok(()),
        Some(_) => Err(ApiError::Unauthorized),
        None => Err(ApiError::NotFound),
    }
}

fn firecrawl_json<T>(payload: Result<Json<T>, JsonRejection>) -> Result<Json<T>, ApiError> {
    payload.map_err(|error| ApiError::InvalidRequest(error.body_text()))
}

#[allow(clippy::too_many_arguments)]
fn validate_firecrawl_scrape_options(
    _only_main_content: Option<bool>,
    remove_base64_images: Option<bool>,
    fast_mode: Option<bool>,
    block_ads: Option<bool>,
    store_in_cache: Option<bool>,
    max_age: Option<u64>,
    mobile: Option<bool>,
) -> Result<(), ApiError> {
    let unsupported = [
        (
            remove_base64_images == Some(false),
            "removeBase64Images=false",
        ),
        (fast_mode == Some(true), "fastMode=true"),
        (block_ads == Some(false), "blockAds=false"),
        (store_in_cache == Some(false), "storeInCache=false"),
        (mobile == Some(true), "mobile=true"),
        (
            max_age.is_some_and(|value| value != 14_400_000),
            "maxAge other than 14400000",
        ),
    ]
    .into_iter()
    .filter_map(|(is_unsupported, name)| is_unsupported.then_some(name))
    .collect::<Vec<_>>();
    if unsupported.is_empty() {
        Ok(())
    } else {
        Err(ApiError::InvalidRequest(format!(
            "BeeCrawl does not support these Firecrawl scrape option values: {}",
            unsupported.join(", ")
        )))
    }
}

fn validate_browser_actions(
    actions: &[crate::models::BrowserAction],
    timeout_ms: u64,
    initial_wait_ms: u64,
) -> Result<(), ApiError> {
    use crate::models::BrowserAction;
    if actions.len() > 50 {
        return Err(ApiError::InvalidRequest(
            "actions must contain at most 50 entries".to_string(),
        ));
    }
    if !(1_000..=300_000).contains(&timeout_ms) {
        return Err(ApiError::InvalidRequest(
            "timeout must be between 1000 and 300000 milliseconds".to_string(),
        ));
    }
    let mut waits = initial_wait_ms;
    let mut payload = 0usize;
    for action in actions {
        match action {
            BrowserAction::Wait {
                milliseconds,
                selector,
            } => {
                if milliseconds.is_some() && selector.is_some() {
                    return Err(ApiError::InvalidRequest(
                        "wait action requires exactly one of milliseconds or selector".to_string(),
                    ));
                }
                waits = waits.saturating_add(milliseconds.unwrap_or_else(|| {
                    if selector.is_none() {
                        1_000
                    } else {
                        0
                    }
                }));
                if selector.as_ref().is_some_and(|value| value.len() > 4_096) {
                    return Err(ApiError::InvalidRequest(
                        "action selector exceeds 4096 bytes".to_string(),
                    ));
                }
            }
            BrowserAction::Click { selector, .. } => {
                if selector.is_empty() || selector.len() > 4_096 {
                    return Err(ApiError::InvalidRequest(
                        "click selector must contain 1 to 4096 bytes".to_string(),
                    ));
                }
            }
            BrowserAction::Write { text } => payload += text.len(),
            BrowserAction::ExecuteJavascript { script } => payload += script.len(),
            BrowserAction::Screenshot {
                quality, viewport, ..
            } => {
                if quality.is_some_and(|value| value == 0 || value > 100) {
                    return Err(ApiError::InvalidRequest(
                        "screenshot quality must be between 1 and 100".to_string(),
                    ));
                }
                if viewport.as_ref().is_some_and(|value| {
                    value.width == 0
                        || value.width > 7_680
                        || value.height == 0
                        || value.height > 4_320
                }) {
                    return Err(ApiError::InvalidRequest(
                        "screenshot viewport exceeds 7680x4320".to_string(),
                    ));
                }
            }
            BrowserAction::Press { key } if key.is_empty() || key.len() > 128 => {
                return Err(ApiError::InvalidRequest(
                    "press key must contain 1 to 128 bytes".to_string(),
                ));
            }
            BrowserAction::Scroll { direction, .. }
                if !matches!(direction.as_str(), "up" | "down") =>
            {
                return Err(ApiError::InvalidRequest(
                    "scroll direction must be up or down".to_string(),
                ));
            }
            BrowserAction::Pdf {
                format: Some(format),
                ..
            } if !matches!(
                format.as_str(),
                "Letter" | "Legal" | "Tabloid" | "A0" | "A1" | "A2" | "A3" | "A4" | "A5"
            ) =>
            {
                return Err(ApiError::InvalidRequest(
                    "pdf format is not supported".to_string(),
                ));
            }
            _ => {}
        }
    }
    if waits > timeout_ms {
        return Err(ApiError::InvalidRequest(
            "action waits exceed the request timeout".to_string(),
        ));
    }
    if payload > 262_144 {
        return Err(ApiError::InvalidRequest(
            "action script and text payloads exceed 262144 bytes".to_string(),
        ));
    }
    Ok(())
}

fn validate_firecrawl_crawl_defaults(request: &FirecrawlV2CrawlRequest) -> Result<(), ApiError> {
    if !matches!(request.sitemap.as_str(), "skip" | "include" | "only") {
        return Err(ApiError::InvalidRequest(
            "sitemap must be one of skip, include, or only".to_string(),
        ));
    }
    let unsupported = [(
        request.zero_data_retention == Some(true),
        "zeroDataRetention=true",
    )]
    .into_iter()
    .filter_map(|(is_unsupported, name)| is_unsupported.then_some(name))
    .collect::<Vec<_>>();
    if unsupported.is_empty() {
        Ok(())
    } else {
        Err(ApiError::InvalidRequest(format!(
            "BeeCrawl does not support these Firecrawl crawl option values: {}",
            unsupported.join(", ")
        )))
    }
}

fn firecrawl_delay_ms(delay: Option<f64>) -> Result<u64, ApiError> {
    match delay {
        None => Ok(0),
        Some(delay) if delay.is_finite() && delay > 0.0 && delay <= 60.0 => {
            Ok((delay * 1_000.0).ceil() as u64)
        }
        Some(_) => Err(ApiError::InvalidRequest(
            "delay must be greater than 0 and no more than 60 seconds".to_string(),
        )),
    }
}

fn firecrawl_max_concurrency(value: Option<usize>) -> Result<usize, ApiError> {
    match value {
        Some(0) => Err(ApiError::InvalidRequest(
            "maxConcurrency must be greater than zero".to_string(),
        )),
        Some(value) if value <= i32::MAX as usize => Ok(value),
        Some(_) => Err(ApiError::InvalidRequest(
            "maxConcurrency is too large".to_string(),
        )),
        None => Ok(10),
    }
}

fn firecrawl_idempotency_key(headers: &HeaderMap) -> Result<Option<String>, ApiError> {
    let Some(value) = headers.get("x-idempotency-key") else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::InvalidRequest("x-idempotency-key must be a UUID".to_string()))?;
    uuid::Uuid::parse_str(value)
        .map_err(|_| ApiError::InvalidRequest("x-idempotency-key must be a UUID".to_string()))?;
    Ok(Some(value.to_string()))
}

fn firecrawl_proxy(mode: Option<&str>) -> Result<Option<ProxyConfig>, ApiError> {
    let mode = mode.unwrap_or("auto");
    if !matches!(mode, "auto" | "basic" | "stealth" | "enhanced") {
        return Err(ApiError::InvalidRequest(
            "proxy must be one of auto, basic, stealth, or enhanced".to_string(),
        ));
    }
    let configured = |name: &str| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    };
    let selected = match mode {
        "basic" => configured("BEECRAWL_PROXY_URL").map(|url| ("basic", url)),
        "stealth" => configured("BEECRAWL_STEALTH_PROXY_URL").map(|url| ("stealth", url)),
        "enhanced" => configured("BEECRAWL_ENHANCED_PROXY_URL")
            .map(|url| ("enhanced", url))
            .or_else(|| configured("BEECRAWL_STEALTH_PROXY_URL").map(|url| ("enhanced", url))),
        "auto" => configured("BEECRAWL_PROXY_URL")
            .map(|url| ("basic", url))
            .or_else(|| configured("BEECRAWL_ENHANCED_PROXY_URL").map(|url| ("enhanced", url)))
            .or_else(|| configured("BEECRAWL_STEALTH_PROXY_URL").map(|url| ("stealth", url))),
        _ => None,
    };
    let Some((selected_mode, raw)) = selected else {
        return if mode == "auto" {
            Ok(None)
        } else {
            Err(ApiError::InvalidRequest(format!(
                "No {mode} proxy is configured"
            )))
        };
    };
    parse_proxy_config(selected_mode, &raw).map(Some)
}

fn parse_proxy_config(mode: &str, raw: &str) -> Result<ProxyConfig, ApiError> {
    let mut url = url::Url::parse(raw)
        .map_err(|error| ApiError::InvalidRequest(format!("Invalid proxy URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https" | "socks5") || url.host_str().is_none() {
        return Err(ApiError::InvalidRequest(
            "Proxy URL must use http, https, or socks5".to_string(),
        ));
    }
    let username = (!url.username().is_empty()).then(|| url.username().to_string());
    let password = url.password().map(str::to_string);
    url.set_username("")
        .map_err(|_| ApiError::InvalidRequest("Proxy username could not be parsed".to_string()))?;
    url.set_password(None)
        .map_err(|_| ApiError::InvalidRequest("Proxy password could not be parsed".to_string()))?;
    Ok(ProxyConfig {
        mode: mode.to_string(),
        server: url.to_string(),
        username,
        password,
    })
}

fn firecrawl_format_names(formats: &[FirecrawlFormat]) -> Vec<String> {
    formats.iter().map(|format| format.name.clone()).collect()
}

fn firecrawl_screenshot_options(
    formats: &[FirecrawlFormat],
) -> Result<Option<ScreenshotOptions>, ApiError> {
    let screenshots = formats
        .iter()
        .filter(|format| format.name() == "screenshot")
        .collect::<Vec<_>>();
    if screenshots.len() > 1 {
        return Err(ApiError::InvalidRequest(
            "Only one screenshot format may be requested".to_string(),
        ));
    }
    let Some(format) = screenshots.first() else {
        return Ok(None);
    };
    let full_page = format
        .option("fullPage")
        .map(|value| {
            value.as_bool().ok_or_else(|| {
                ApiError::InvalidRequest("screenshot.fullPage must be a boolean".to_string())
            })
        })
        .transpose()?
        .unwrap_or(false);
    let quality = format
        .option("quality")
        .map(|value| {
            value
                .as_u64()
                .filter(|quality| (1..=100).contains(quality))
                .map(|quality| quality as u8)
                .ok_or_else(|| {
                    ApiError::InvalidRequest(
                        "screenshot.quality must be between 1 and 100".to_string(),
                    )
                })
        })
        .transpose()?;
    let viewport = format
        .option("viewport")
        .map(|value| {
            let width = value.get("width").and_then(serde_json::Value::as_u64);
            let height = value.get("height").and_then(serde_json::Value::as_u64);
            match (width, height) {
                (Some(width @ 1..=7680), Some(height @ 1..=4320)) => Ok(ScreenshotViewport {
                    width: width as u32,
                    height: height as u32,
                }),
                _ => Err(ApiError::InvalidRequest(
                    "screenshot.viewport must contain width 1..7680 and height 1..4320".to_string(),
                )),
            }
        })
        .transpose()?;
    let unsupported = format
        .options
        .keys()
        .filter(|key| !matches!(key.as_str(), "fullPage" | "quality" | "viewport"))
        .cloned()
        .collect::<Vec<_>>();
    if !unsupported.is_empty() {
        return Err(ApiError::InvalidRequest(format!(
            "Unsupported screenshot options: {}",
            unsupported.join(", ")
        )));
    }
    Ok(Some(ScreenshotOptions {
        full_page,
        quality,
        viewport,
    }))
}

async fn enrich_firecrawl_document(
    state: &AppState,
    source_url: &str,
    formats: &[FirecrawlFormat],
    requested_raw_html: bool,
    document: &mut serde_json::Value,
) -> Result<(), ApiError> {
    let markdown = document
        .get("markdown")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let raw_html = document
        .get("rawHtml")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let Some(fields) = document.as_object_mut() else {
        return Ok(());
    };

    for format in formats {
        match format.name() {
            "images" => {
                let images = raw_html
                    .as_deref()
                    .map(|html| web_extract::extract_images(html, source_url))
                    .unwrap_or_default();
                fields.insert("images".to_string(), json!(images));
            }
            "summary" => {
                fields.insert("summary".to_string(), json!(summarize_markdown(&markdown)));
            }
            "attributes" => {
                let selectors = firecrawl_attribute_selectors(format)?;
                let attributes = raw_html
                    .as_deref()
                    .map(|html| extract_attributes(html, &selectors))
                    .transpose()?
                    .unwrap_or_default();
                fields.insert("attributes".to_string(), json!(attributes));
            }
            "question" => {
                let question = required_format_string(format, "question")?;
                let answer =
                    answer_firecrawl_query(state, source_url, &markdown, question, "answer")
                        .await?;
                fields.insert("answer".to_string(), json!(answer));
            }
            "highlights" => {
                let query = required_format_string(format, "query")?;
                let highlights =
                    answer_firecrawl_query(state, source_url, &markdown, query, "highlights")
                        .await?;
                fields.insert("highlights".to_string(), json!(highlights));
            }
            "json" | "deterministicJson" => {
                let schema = format
                    .option("schema")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let prompt = format.option("prompt").and_then(serde_json::Value::as_str);
                let value = if format.name() == "json" {
                    if let Some(provider) = llm::resolve_provider(None)? {
                        llm::extract_structured_value(
                            &state.client,
                            &provider,
                            &[source_url.to_string()],
                            &schema,
                            &markdown,
                            prompt,
                        )
                        .await?
                    } else {
                        deterministic_json(&schema, &markdown, document_title(fields))
                    }
                } else {
                    deterministic_json(&schema, &markdown, document_title(fields))
                };
                fields.insert("json".to_string(), value);
            }
            _ => {}
        }
    }
    if !requested_raw_html {
        fields.remove("rawHtml");
    }
    Ok(())
}

fn validate_firecrawl_enrichment_formats(formats: &[FirecrawlFormat]) -> Result<(), ApiError> {
    for format in formats {
        let allowed: &[&str] = match format.name() {
            "attributes" => &["selectors"],
            "question" => &["question"],
            "highlights" => &["query"],
            _ => continue,
        };
        let unsupported = format
            .options
            .keys()
            .filter(|key| !allowed.contains(&key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unsupported.is_empty() {
            return Err(ApiError::InvalidRequest(format!(
                "Unsupported {} options: {}",
                format.name(),
                unsupported.join(", ")
            )));
        }
        match format.name() {
            "attributes" => {
                firecrawl_attribute_selectors(format)?;
            }
            "question" => {
                required_format_string(format, "question")?;
            }
            "highlights" => {
                required_format_string(format, "query")?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn required_format_string<'a>(
    format: &'a FirecrawlFormat,
    option: &str,
) -> Result<&'a str, ApiError> {
    format
        .option(option)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty() && value.chars().count() <= 10_000)
        .ok_or_else(|| {
            ApiError::InvalidRequest(format!(
                "{}.{} must be a non-empty string of at most 10000 characters",
                format.name(),
                option
            ))
        })
}

#[derive(serde::Serialize)]
struct FirecrawlAttributeResult {
    selector: String,
    attribute: String,
    values: Vec<String>,
}

fn firecrawl_attribute_selectors(
    format: &FirecrawlFormat,
) -> Result<Vec<(String, String)>, ApiError> {
    let selectors = format
        .option("selectors")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            ApiError::InvalidRequest("attributes.selectors must be an array".to_string())
        })?;
    selectors
        .iter()
        .map(|selector| {
            let object = selector.as_object().ok_or_else(|| {
                ApiError::InvalidRequest("Each attributes selector must be an object".to_string())
            })?;
            if object.len() != 2
                || !object.contains_key("selector")
                || !object.contains_key("attribute")
            {
                return Err(ApiError::InvalidRequest(
                    "Each attributes selector must contain only selector and attribute".to_string(),
                ));
            }
            let css = object
                .get("selector")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApiError::InvalidRequest(
                        "attributes selector must be a non-empty string".to_string(),
                    )
                })?;
            let attribute = object
                .get("attribute")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApiError::InvalidRequest(
                        "attributes attribute must be a non-empty string".to_string(),
                    )
                })?;
            scraper::Selector::parse(css)
                .map_err(|_| ApiError::InvalidRequest(format!("Invalid CSS selector: {css}")))?;
            Ok((css.to_string(), attribute.to_string()))
        })
        .collect()
}

fn extract_attributes(
    html: &str,
    selectors: &[(String, String)],
) -> Result<Vec<FirecrawlAttributeResult>, ApiError> {
    let document = scraper::Html::parse_document(html);
    selectors
        .iter()
        .map(|(css, attribute)| {
            let selector = scraper::Selector::parse(css)
                .map_err(|_| ApiError::InvalidRequest(format!("Invalid CSS selector: {css}")))?;
            let data_attribute =
                (!attribute.starts_with("data-")).then(|| format!("data-{attribute}"));
            let values = document
                .select(&selector)
                .filter_map(|element| {
                    element.value().attr(attribute).or_else(|| {
                        data_attribute
                            .as_deref()
                            .and_then(|name| element.value().attr(name))
                    })
                })
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect();
            Ok(FirecrawlAttributeResult {
                selector: css.clone(),
                attribute: attribute.clone(),
                values,
            })
        })
        .collect()
}

async fn answer_firecrawl_query(
    state: &AppState,
    source_url: &str,
    markdown: &str,
    query: &str,
    field: &str,
) -> Result<String, ApiError> {
    if let Some(provider) = llm::resolve_provider(None)? {
        let schema = json!({
            "type": "object",
            "properties": { (field): { "type": "string" } },
            "required": [field]
        });
        let instructions = if field == "answer" {
            format!("Answer this question using only the page content: {query}")
        } else {
            format!("Return the page passages most relevant to this query: {query}")
        };
        let value = llm::extract_structured_value(
            &state.client,
            &provider,
            &[source_url.to_string()],
            &schema,
            markdown,
            Some(&instructions),
        )
        .await?;
        if let Some(result) = value.get(field).and_then(serde_json::Value::as_str) {
            return Ok(result.to_string());
        }
    }
    Ok(relevant_markdown_passage(markdown, query))
}

fn relevant_markdown_passage(markdown: &str, query: &str) -> String {
    let terms = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() >= 3)
        .map(str::to_lowercase)
        .collect::<std::collections::HashSet<_>>();
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|passage| !passage.is_empty() && !passage.starts_with('#'))
        .max_by_key(|passage| {
            let lowercase = passage.to_lowercase();
            terms
                .iter()
                .filter(|term| lowercase.contains(*term))
                .count()
        })
        .unwrap_or_else(|| markdown.trim())
        .chars()
        .take(2_000)
        .collect()
}

fn deterministic_json(
    schema: &serde_json::Value,
    markdown: &str,
    title: Option<String>,
) -> serde_json::Value {
    let fields = firecrawl_extract_schema(schema);
    serde_json::to_value(
        fields
            .keys()
            .map(|field| (field.clone(), extract_field(field, markdown, title.clone())))
            .collect::<HashMap<_, _>>(),
    )
    .unwrap_or_else(|_| json!({}))
}

fn document_title(fields: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    fields
        .get("metadata")
        .and_then(|metadata| metadata.get("title"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn summarize_markdown(markdown: &str) -> String {
    markdown
        .split("\n\n")
        .map(str::trim)
        .find(|section| !section.is_empty() && !section.starts_with('#'))
        .unwrap_or_else(|| markdown.trim())
        .chars()
        .take(500)
        .collect()
}

fn firecrawl_search_result(result: crate::models::SearchResult) -> serde_json::Value {
    if let Some(markdown) = result.markdown {
        json!({
            "url": result.url,
            "title": result.title,
            "description": result.description,
            "markdown": markdown,
            "highlights": result.highlights,
            "metadata": {
                "title": result.title,
                "description": result.description,
                "sourceURL": result.url,
                "url": result.metadata.get("final_url"),
                "category": result.metadata.get("category"),
            }
        })
    } else {
        json!({
            "url": result.url,
            "title": result.title,
            "description": result.description,
            "highlights": result.highlights,
            "category": result.metadata.get("category"),
        })
    }
}

fn firecrawl_map_links(links: Vec<String>) -> Vec<serde_json::Value> {
    links.into_iter().map(|url| json!({ "url": url })).collect()
}

fn firecrawl_document(response: WebExtractScrapeResponse) -> serde_json::Value {
    json!({
        "markdown": response.markdown,
        "html": response.html,
        "rawHtml": response.raw_html,
        "links": response.links,
        "screenshot": response.screenshot,
        "actions": response.actions,
        "metadata": {
            "title": response.metadata.title,
            "language": response.metadata.language,
            "sourceURL": response.url,
            "url": response.final_url,
            "statusCode": response.metadata.status_code,
            "scrapeId": response.request_id,
            "engine": response.metadata.provider,
            "engineOutcomes": response.metadata.engine_outcomes,
            "fallbackReason": response.metadata.fallback_reason,
            "proxyUsed": response.metadata.proxy_used,
        }
    })
}

fn firecrawl_crawl_status(response: CrawlStatusResponse, resource: &str) -> serde_json::Value {
    let next = response.pagination.next.map(|offset| {
        format!(
            "/v2/{resource}/{}?skip={offset}&limit={}",
            response.id, response.pagination.limit
        )
    });
    json!({
        "success": true,
        "id": response.id,
        "status": firecrawl_status(&response.status),
        "total": response.total,
        "completed": response.completed,
        "creditsUsed": response.completed,
        "expiresAt": response.expires_at,
        "data": response.data.into_iter().map(firecrawl_document).collect::<Vec<_>>(),
        "next": next,
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
        })
        .or_else(|| {
            headers
                .get("sec-websocket-protocol")
                .and_then(|value| value.to_str().ok())
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
    Workflow(workflows::WorkflowError),
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

impl From<workflows::WorkflowError> for ApiError {
    fn from(value: workflows::WorkflowError) -> Self {
        match value {
            workflows::WorkflowError::NotFound => Self::NotFound,
            other => Self::Workflow(other),
        }
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
            Self::Workflow(error) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "detail": {
                        "code": "workflow_storage_error",
                        "message": error.to_string(),
                        "retryable": true
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
    fn firecrawl_v2_batch_accepts_scrape_options() {
        let request: FirecrawlV2BatchScrapeRequest = serde_json::from_value(json!({
            "urls": ["https://example.com", "https://example.com/docs"],
            "formats": ["markdown"],
            "waitFor": 250,
            "timeout": 45000,
            "maxConcurrency": 4
        }))
        .unwrap();
        assert_eq!(request.urls.len(), 2);
        assert_eq!(
            firecrawl_format_names(&request.scrape_options.formats),
            ["markdown"]
        );
        assert_eq!(request.scrape_options.wait_for_ms, 250);
        assert_eq!(request.scrape_options.timeout, 45_000);
        assert_eq!(request.max_concurrency, Some(4));
    }

    #[test]
    fn firecrawl_v2_defaults_match_current_sdk_contract() {
        let crawl: FirecrawlV2CrawlRequest = serde_json::from_value(json!({
            "url": "https://example.com"
        }))
        .unwrap();
        assert_eq!(crawl.limit, 10_000);
        assert_eq!(crawl.max_discovery_depth, 10_000);
        assert!(!crawl.ignore_query_parameters);
        assert!(crawl.include_paths.is_empty());
        assert!(crawl.exclude_paths.is_empty());
        assert_eq!(crawl.regex_on_full_url, None);
        assert_eq!(crawl.sitemap, "include");
        assert_eq!(crawl.allow_external_links, None);
        assert_eq!(crawl.crawl_entire_domain, None);
        assert_eq!(crawl.delay, None);
        assert_eq!(crawl.max_concurrency, None);
        assert_eq!(crawl.deduplicate_similar_urls, None);
        assert!(crawl.webhook.is_none());

        let webhook_crawl: FirecrawlV2CrawlRequest = serde_json::from_value(json!({
            "url": "https://example.com",
            "webhook": {
                "url": "https://hooks.example.com/crawl",
                "headers": { "X-Tenant": "bee" },
                "metadata": { "environment": "test" },
                "events": ["started", "page", "completed"]
            }
        }))
        .unwrap();
        let webhook = webhook_crawl.webhook.unwrap().config();
        assert_eq!(webhook.headers["X-Tenant"], "bee");
        assert_eq!(webhook.metadata["environment"], "test");

        let proxy = parse_proxy_config(
            "basic",
            "http://proxy-user:proxy-pass@proxy.example.com:8080",
        )
        .unwrap();
        assert_eq!(proxy.server, "http://proxy.example.com:8080/");
        assert_eq!(proxy.username.as_deref(), Some("proxy-user"));
        assert_eq!(proxy.password.as_deref(), Some("proxy-pass"));

        assert_eq!(firecrawl_delay_ms(Some(0.25)).unwrap(), 250);
        assert_eq!(firecrawl_max_concurrency(Some(3)).unwrap(), 3);
        assert!(firecrawl_delay_ms(Some(0.0)).is_err());
        assert!(firecrawl_delay_ms(Some(60.1)).is_err());
        assert!(firecrawl_max_concurrency(Some(0)).is_err());

        let map: FirecrawlV2MapRequest = serde_json::from_value(json!({
            "url": "https://example.com"
        }))
        .unwrap();
        assert_eq!(map.limit, 5_000);
        assert!(map.include_subdomains);

        let search: FirecrawlV2SearchRequest = serde_json::from_value(json!({
            "query": "beecrawl"
        }))
        .unwrap();
        assert_eq!(search.limit, 10);
    }

    #[test]
    fn firecrawl_v2_map_returns_link_objects() {
        let links = firecrawl_map_links(vec![
            "https://example.com/".to_string(),
            "https://example.com/docs".to_string(),
        ]);
        assert_eq!(links[0], json!({ "url": "https://example.com/" }));
        assert_eq!(links[1]["url"], "https://example.com/docs");
    }

    #[tokio::test]
    async fn firecrawl_v2_batch_route_uses_async_storage() {
        let app = app_with_crawls(CrawlStore::unavailable("Postgres is not configured"));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v2/batch/scrape")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"urls":["https://example.com"],"formats":["markdown"]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn firecrawl_status_includes_pagination_url_and_expiry() {
        let response = firecrawl_crawl_status(
            CrawlStatusResponse {
                id: "job-id".to_string(),
                url: "https://example.com".to_string(),
                status: "scraping".to_string(),
                total: 3,
                completed: 2,
                failed: 0,
                data: Vec::new(),
                errors: Vec::new(),
                pagination: crate::models::CrawlPagination {
                    offset: 0,
                    limit: 2,
                    total: 3,
                    next: Some(2),
                },
                expires_at: Some("2026-07-25T00:00:00+00:00".to_string()),
            },
            "crawl",
        );
        assert_eq!(response["id"], "job-id");
        assert_eq!(response["expiresAt"], "2026-07-25T00:00:00+00:00");
        assert_eq!(response["next"], "/v2/crawl/job-id?skip=2&limit=2");
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
            "categories": ["github", { "type": "research", "sites": ["arxiv.org"] }, "pdf"],
            "includeDomains": ["example.com"],
            "lang": "de",
            "country": "de",
            "location": "Berlin",
            "tbs": "qdr:w",
            "asyncScraping": true,
            "highlights": true,
            "scrapeOptions": { "formats": [{ "type": "markdown" }], "timeout": 45000 }
        }))
        .unwrap();
        assert_eq!(request.sources[0].name(), "web");
        assert_eq!(request.sources[1].name(), "news");
        assert_eq!(request.categories[0].name(), "github");
        assert_eq!(request.categories[1].sites(), ["arxiv.org"]);
        assert_eq!(request.include_domains, ["example.com"]);
        assert!(request.async_scraping && request.highlights);
        let scrape_options = request.scrape_options.unwrap();
        assert_eq!(
            firecrawl_format_names(&scrape_options.formats),
            ["markdown"]
        );
        assert_eq!(scrape_options.timeout, 45_000);
    }

    #[test]
    fn firecrawl_v2_formats_accept_strings_and_optionless_objects() {
        let request: FirecrawlV2ScrapeRequest = serde_json::from_value(json!({
            "url": "https://example.com",
            "formats": ["html", { "type": "markdown" }, { "type": "screenshot" }]
        }))
        .unwrap();
        assert_eq!(
            firecrawl_format_names(&request.formats),
            ["html", "markdown", "screenshot"]
        );
    }

    #[test]
    fn firecrawl_v2_preserves_format_options() {
        let request = serde_json::from_value::<FirecrawlV2ScrapeRequest>(json!({
            "url": "https://example.com",
            "formats": [{ "type": "json", "schema": { "type": "object" }, "prompt": "Extract" }]
        }))
        .unwrap();
        assert_eq!(request.formats[0].name(), "json");
        assert_eq!(request.formats[0].option("prompt"), Some(&json!("Extract")));
    }

    #[test]
    fn firecrawl_v2_validates_screenshot_options() {
        let request = serde_json::from_value::<FirecrawlV2ScrapeRequest>(json!({
            "url": "https://example.com",
            "formats": [{
                "type": "screenshot",
                "fullPage": true,
                "quality": 85,
                "viewport": { "width": 1440, "height": 900 }
            }]
        }))
        .unwrap();
        let options = firecrawl_screenshot_options(&request.formats)
            .unwrap()
            .unwrap();
        assert!(options.full_page);
        assert_eq!(options.quality, Some(85));
        assert_eq!(options.viewport.unwrap().width, 1440);

        let invalid = serde_json::from_value::<FirecrawlV2ScrapeRequest>(json!({
            "url": "https://example.com",
            "formats": [{ "type": "screenshot", "quality": 0 }]
        }))
        .unwrap();
        assert!(firecrawl_screenshot_options(&invalid.formats).is_err());
    }

    #[tokio::test]
    async fn firecrawl_v2_enriches_images_summary_and_deterministic_json() {
        let request = serde_json::from_value::<FirecrawlV2ScrapeRequest>(json!({
            "url": "https://example.com",
            "formats": [
                "images",
                "summary",
                { "type": "attributes", "selectors": [
                    { "selector": "#hero", "attribute": "data-name" },
                    { "selector": "img", "attribute": "alt" }
                ]},
                { "type": "deterministicJson", "schema": {
                    "type": "object",
                    "properties": { "title": { "type": "string" } }
                }}
            ]
        }))
        .unwrap();
        let state = AppState {
            client: reqwest::Client::new(),
            cache: CacheStore::from_env(),
            crawls: CrawlStore::unavailable("not needed"),
            workflows: WorkflowStore::default(),
            parse_uploads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            browser_owners: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            scrape_handoffs: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        };
        let mut document = json!({
            "markdown": "# Example\n\nA useful summary paragraph.",
            "rawHtml": "<html><body><img id='hero' src='/hero.png' alt='Bee' data-name='worker'></body></html>",
            "metadata": { "title": "Example" }
        });
        enrich_firecrawl_document(
            &state,
            "https://example.com/page",
            &request.formats,
            false,
            &mut document,
        )
        .await
        .unwrap();
        assert_eq!(document["images"][0], "https://example.com/hero.png");
        assert_eq!(document["summary"], "A useful summary paragraph.");
        assert_eq!(document["json"]["title"], "Example");
        assert_eq!(document["attributes"][0]["values"][0], "worker");
        assert_eq!(document["attributes"][1]["values"][0], "Bee");
        assert!(document.get("rawHtml").is_none());
    }

    #[test]
    fn firecrawl_v2_validates_structured_format_options() {
        let request = serde_json::from_value::<FirecrawlV2ScrapeRequest>(json!({
            "url": "https://example.com",
            "formats": [
                { "type": "question", "question": "What is BeeCrawl?" },
                { "type": "highlights", "query": "browser scraping" },
                { "type": "attributes", "selectors": [
                    { "selector": "a[href]", "attribute": "href" }
                ]}
            ]
        }))
        .unwrap();
        validate_firecrawl_enrichment_formats(&request.formats).unwrap();

        for format in [
            json!({ "type": "question", "question": "" }),
            json!({ "type": "highlights", "query": "ok", "ignored": true }),
            json!({ "type": "attributes", "selectors": [
                { "selector": "[", "attribute": "href" }
            ]}),
        ] {
            let request = serde_json::from_value::<FirecrawlV2ScrapeRequest>(json!({
                "url": "https://example.com",
                "formats": [format]
            }))
            .unwrap();
            assert!(validate_firecrawl_enrichment_formats(&request.formats).is_err());
        }
    }

    #[test]
    fn firecrawl_query_fallback_selects_the_most_relevant_passage() {
        let markdown = "# BeeCrawl\n\nA general introduction.\n\nBrowser scraping renders JavaScript pages reliably.";
        assert_eq!(
            relevant_markdown_passage(markdown, "How does browser scraping work?"),
            "Browser scraping renders JavaScript pages reliably."
        );
    }

    #[test]
    fn firecrawl_v2_rejects_unsupported_formats() {
        let error = serde_json::from_value::<FirecrawlV2ScrapeRequest>(json!({
            "url": "https://example.com",
            "formats": [{ "type": "audio" }]
        }))
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("format 'audio' is not supported"));
    }

    #[test]
    fn firecrawl_v2_accepts_and_validates_browser_actions() {
        let request = serde_json::from_value::<FirecrawlV2ScrapeRequest>(json!({
            "url": "https://example.com",
            "formats": ["markdown"],
            "actions": [
                { "type": "wait", "selector": "#ready" },
                { "type": "click", "selector": "button" },
                { "type": "write", "text": "hello" },
                { "type": "press", "key": "Enter" },
                { "type": "scroll", "direction": "down" },
                { "type": "scrape" },
                { "type": "executeJavascript", "script": "document.title" },
                { "type": "pdf", "format": "A4" }
            ]
        }))
        .unwrap();
        assert_eq!(request.actions.len(), 8);
        validate_browser_actions(&request.actions, request.timeout, request.wait_for_ms).unwrap();

        let invalid = vec![crate::models::BrowserAction::Wait {
            milliseconds: Some(300_001),
            selector: None,
        }];
        assert!(validate_browser_actions(&invalid, 300_000, 0).is_err());
    }

    #[tokio::test]
    async fn firecrawl_v2_contract_errors_are_json_400_responses() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v2/scrape")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"url":"https://example.com","formats":[{"type":"audio"}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["success"], false);
        assert!(error["error"]
            .as_str()
            .unwrap()
            .contains("format 'audio' is not supported"));
    }

    #[tokio::test]
    async fn firecrawl_v2_rejects_invalid_idempotency_keys_before_enqueue() {
        let response = app_with_crawls(CrawlStore::unavailable("not needed"))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v2/crawl")
                    .header("content-type", "application/json")
                    .header("x-idempotency-key", "not-a-uuid")
                    .body(Body::from(r#"{"url":"https://example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn firecrawl_v2_accepts_official_sdk_scrape_defaults() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v2/scrape")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"url":"http://127.0.0.1:1","onlyMainContent":true,"skipTlsVerification":true,"removeBase64Images":true,"fastMode":false,"blockAds":true,"storeInCache":true,"maxAge":14400000,"formats":["markdown"],"mobile":false,"origin":"python-sdk"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn firecrawl_v2_rejects_unsupported_semantic_option_values() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v2/scrape")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"url":"https://example.com","formats":["markdown"],"mobile":true,"fastMode":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(error["error"].as_str().unwrap().contains("mobile=true"));
        assert!(error["error"].as_str().unwrap().contains("fastMode=true"));
    }

    #[tokio::test]
    async fn firecrawl_v2_crawl_rejects_non_markdown_formats_before_enqueue() {
        let app = app_with_crawls(CrawlStore::unavailable("Postgres is not configured"));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v2/crawl")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"url":"https://example.com","scrapeOptions":{"formats":["html"]}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(error["error"]
            .as_str()
            .unwrap()
            .contains("only the markdown scrape format"));
    }

    #[test]
    fn firecrawl_v2_parse_accepts_pdf_parser_options() {
        let options: FirecrawlV2ParseOptions = serde_json::from_value(json!({
            "formats": [{ "type": "markdown" }],
            "timeout": 120000,
            "parsers": [{ "type": "pdf", "mode": "fast", "maxPages": 12 }]
        }))
        .unwrap();
        assert_eq!(firecrawl_format_names(&options.formats), ["markdown"]);
        assert_eq!(options.timeout, 120_000);
        assert_eq!(options.parsers[0].kind, "pdf");
        assert_eq!(options.parsers[0].mode.as_deref(), Some("fast"));
        assert_eq!(options.parsers[0].max_pages, Some(12));
    }

    #[test]
    fn parse_base64_accepts_bare_and_data_url_values() {
        let bare = decode_base64_document("JVBERi0=").unwrap();
        let data_url = decode_base64_document("data:application/pdf;base64,JVBERi0=").unwrap();
        assert_eq!(bare, b"%PDF-");
        assert_eq!(data_url, bare);
    }

    #[tokio::test]
    async fn parse_upload_references_are_expiring_and_single_use() {
        let state = AppState {
            client: reqwest::Client::new(),
            cache: CacheStore::from_env(),
            crawls: CrawlStore::unavailable("not needed"),
            workflows: WorkflowStore::default(),
            parse_uploads: Arc::new(tokio::sync::Mutex::new(HashMap::from([(
                "upload-token".to_string(),
                ParseUpload {
                    filename: "upload.html".to_string(),
                    expires_at: chrono::Utc::now() + chrono::Duration::minutes(1),
                    data: Some(Bytes::from_static(b"<h1>Upload</h1>")),
                },
            )]))),
            browser_owners: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            scrape_handoffs: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        };
        let upload = take_parse_upload(&state, "upload-token").await.unwrap();
        assert_eq!(upload.filename, "upload.html");
        assert_eq!(upload.data.unwrap(), Bytes::from_static(b"<h1>Upload</h1>"));
        assert!(take_parse_upload(&state, "upload-token").await.is_err());
    }

    #[tokio::test]
    async fn browser_sessions_are_scoped_to_the_calling_key() {
        let state = AppState {
            client: reqwest::Client::new(),
            cache: CacheStore::from_env(),
            crawls: CrawlStore::unavailable("not needed"),
            workflows: WorkflowStore::default(),
            parse_uploads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            browser_owners: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            scrape_handoffs: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        };
        let first = HeaderMap::from_iter([(
            axum::http::header::AUTHORIZATION,
            "Bearer first".parse().unwrap(),
        )]);
        let second = HeaderMap::from_iter([(
            axum::http::header::AUTHORIZATION,
            "Bearer second".parse().unwrap(),
        )]);
        state
            .browser_owners
            .lock()
            .await
            .insert("session".to_string(), browser_owner(&first));
        assert!(require_browser_owner(&state, &first, "session")
            .await
            .is_ok());
        assert!(matches!(
            require_browser_owner(&state, &second, "session").await,
            Err(ApiError::Unauthorized)
        ));

        let create: BrowserCreateRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!((create.ttl, create.activity_ttl), (600, 300));
        let execute: BrowserExecuteRequest = serde_json::from_value(json!({
            "code": "document.title"
        }))
        .unwrap();
        assert_eq!(execute.language, "node");
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
            highlights: vec!["Example highlight".to_string()],
        });
        assert_eq!(result["markdown"], "# Example");
        assert_eq!(result["title"], "Example");
        assert_eq!(result["description"], "Description");
        assert_eq!(result["url"], "https://example.com");
        assert_eq!(result["metadata"]["sourceURL"], "https://example.com");
    }
}
