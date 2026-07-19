use reqwest::{Client as HttpClient, Method, StatusCode};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BeeCrawlError {
    #[error("BeeCrawl request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("BeeCrawl returned invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("BeeCrawl request failed with status {status}: {message}")]
    Api {
        status: StatusCode,
        message: String,
        detail: Value,
    },
    #[error("timed out waiting for job {job_id}")]
    PollTimeout { job_id: String },
    #[error("invalid base URL: {0}")]
    InvalidBaseUrl(String),
}

pub type Result<T> = std::result::Result<T, BeeCrawlError>;

#[derive(Clone, Debug)]
pub struct BeeCrawlClient {
    http: HttpClient,
    base_url: String,
    api_key: Option<String>,
}

impl BeeCrawlClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::builder(base_url)
            .build()
            .expect("default client configuration is valid")
    }

    pub fn builder(base_url: impl Into<String>) -> BeeCrawlClientBuilder {
        BeeCrawlClientBuilder {
            base_url: base_url.into(),
            api_key: None,
            timeout: Duration::from_secs(60),
        }
    }

    pub async fn scrape(&self, url: &str, options: Value) -> Result<Value> {
        self.post("/scrape", merge_url(options, url)).await
    }

    pub async fn map(&self, url: &str, options: Value) -> Result<Value> {
        self.post("/map", merge_url(options, url)).await
    }

    pub async fn search(&self, query: &str, options: Value) -> Result<Value> {
        self.post("/search", merge_field(options, "query", query))
            .await
    }

    pub async fn extract(
        &self,
        url: &str,
        schema: HashMap<String, String>,
        options: Value,
    ) -> Result<Value> {
        let mut payload = merge_url(options, url);
        payload["schema"] = json!(schema);
        self.post("/extract", payload).await
    }

    pub async fn crawl(&self, url: &str, options: Value) -> Result<Value> {
        self.post("/crawl", merge_url(options, url)).await
    }

    pub async fn batch_scrape(&self, urls: Vec<String>, options: Value) -> Result<Value> {
        self.post("/batch/scrape", merge_field(options, "urls", urls))
            .await
    }

    pub async fn crawl_status(&self, job_id: &str, offset: usize, limit: usize) -> Result<Value> {
        self.get(
            &format!("/crawl/{job_id}"),
            &[("offset", offset.to_string()), ("limit", limit.to_string())],
        )
        .await
    }

    pub async fn batch_scrape_status(
        &self,
        job_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Value> {
        self.get(
            &format!("/batch/scrape/{job_id}"),
            &[("offset", offset.to_string()), ("limit", limit.to_string())],
        )
        .await
    }

    pub async fn cancel_crawl(&self, job_id: &str) -> Result<Value> {
        self.request(Method::DELETE, &format!("/crawl/{job_id}"), None, &[])
            .await
    }

    pub async fn cancel_batch_scrape(&self, job_id: &str) -> Result<Value> {
        self.request(
            Method::DELETE,
            &format!("/batch/scrape/{job_id}"),
            None,
            &[],
        )
        .await
    }

    pub async fn v2_scrape(&self, url: &str, options: Value) -> Result<Value> {
        self.post("/v2/scrape", merge_url(options, url)).await
    }
    pub async fn v2_map(&self, url: &str, options: Value) -> Result<Value> {
        self.post("/v2/map", merge_url(options, url)).await
    }
    pub async fn v2_search(&self, query: &str, options: Value) -> Result<Value> {
        self.post("/v2/search", merge_field(options, "query", query))
            .await
    }
    pub async fn v2_extract(&self, urls: Vec<String>, options: Value) -> Result<Value> {
        self.post("/v2/extract", merge_field(options, "urls", urls))
            .await
    }
    pub async fn v2_parse_base64(
        &self,
        data: &str,
        filename: &str,
        options: Value,
    ) -> Result<Value> {
        self.post(
            "/v2/parse/base64",
            merge_field(merge_field(options, "base64", data), "filename", filename),
        )
        .await
    }
    pub async fn v2_parse_reference(&self, upload_ref: &str, options: Value) -> Result<Value> {
        self.post(
            "/v2/parse/reference",
            merge_field(options, "uploadRef", upload_ref),
        )
        .await
    }
    pub async fn create_parse_upload(&self, filename: &str) -> Result<Value> {
        self.post("/v2/parse/upload-url", json!({"filename": filename}))
            .await
    }
    pub async fn upload_parse_document(&self, upload_ref: &str, data: Vec<u8>) -> Result<Value> {
        self.request_bytes(Method::PUT, &format!("/v2/parse/upload/{upload_ref}"), data)
            .await
    }
    pub async fn v2_parse(&self, filename: &str, data: Vec<u8>, options: Value) -> Result<Value> {
        let form = reqwest::multipart::Form::new()
            .text("options", options.to_string())
            .part(
                "file",
                reqwest::multipart::Part::bytes(data).file_name(filename.to_string()),
            );
        self.request_form("/v2/parse", form).await
    }
    pub async fn v2_crawl(&self, url: &str, options: Value) -> Result<Value> {
        self.post("/v2/crawl", merge_url(options, url)).await
    }
    pub async fn v2_batch_scrape(&self, urls: Vec<String>, options: Value) -> Result<Value> {
        self.post("/v2/batch/scrape", merge_field(options, "urls", urls))
            .await
    }
    pub async fn v2_job_status(
        &self,
        kind: &str,
        job_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Value> {
        self.get(
            &format!("/v2/{kind}/{job_id}"),
            &[("offset", offset.to_string()), ("limit", limit.to_string())],
        )
        .await
    }
    pub async fn v2_job_errors(&self, kind: &str, job_id: &str) -> Result<Value> {
        self.get(&format!("/v2/{kind}/{job_id}/errors"), &[]).await
    }
    pub async fn cancel_v2_job(&self, kind: &str, job_id: &str) -> Result<Value> {
        self.request(Method::DELETE, &format!("/v2/{kind}/{job_id}"), None, &[])
            .await
    }
    pub async fn active_crawls(&self) -> Result<Value> {
        self.get("/v2/crawl/active", &[]).await
    }
    pub async fn create_browser_session(&self, options: Value) -> Result<Value> {
        self.post("/v2/browser", options).await
    }
    pub async fn browser_sessions(&self) -> Result<Value> {
        self.get("/v2/browser", &[]).await
    }
    pub async fn execute_browser(
        &self,
        session_id: &str,
        code: &str,
        options: Value,
    ) -> Result<Value> {
        self.post(
            &format!("/v2/browser/{session_id}/execute"),
            merge_field(options, "code", code),
        )
        .await
    }
    pub async fn browser_replay(&self, session_id: &str, page_id: Option<&str>) -> Result<Value> {
        self.get(
            &format!(
                "/v2/browser/{session_id}/replay{}",
                page_id.map(|id| format!("/{id}")).unwrap_or_default()
            ),
            &[],
        )
        .await
    }
    pub async fn delete_browser_session(&self, session_id: &str) -> Result<Value> {
        self.request(
            Method::DELETE,
            &format!("/v2/browser/{session_id}"),
            None,
            &[],
        )
        .await
    }
    pub async fn interact_with_scrape(&self, scrape_id: &str, options: Value) -> Result<Value> {
        self.post(&format!("/v2/scrape/{scrape_id}/interact"), options)
            .await
    }
    pub async fn delete_scrape_interaction(&self, scrape_id: &str) -> Result<Value> {
        self.request(
            Method::DELETE,
            &format!("/v2/scrape/{scrape_id}/interact"),
            None,
            &[],
        )
        .await
    }
    pub async fn create_agent(&self, prompt: &str, options: Value) -> Result<Value> {
        self.post("/v2/agent", merge_field(options, "prompt", prompt))
            .await
    }
    pub async fn get_agent(&self, job_id: &str) -> Result<Value> {
        self.get(&format!("/v2/agent/{job_id}"), &[]).await
    }
    pub async fn cancel_agent(&self, job_id: &str) -> Result<Value> {
        self.request(Method::DELETE, &format!("/v2/agent/{job_id}"), None, &[])
            .await
    }
    pub async fn create_monitor(&self, payload: Value) -> Result<Value> {
        self.post("/v2/monitor", payload).await
    }
    pub async fn list_monitors(&self) -> Result<Value> {
        self.get("/v2/monitor", &[]).await
    }
    pub async fn get_monitor(&self, monitor_id: &str) -> Result<Value> {
        self.get(&format!("/v2/monitor/{monitor_id}"), &[]).await
    }
    pub async fn update_monitor(&self, monitor_id: &str, payload: Value) -> Result<Value> {
        self.request(
            Method::PATCH,
            &format!("/v2/monitor/{monitor_id}"),
            Some(payload),
            &[],
        )
        .await
    }
    pub async fn delete_monitor(&self, monitor_id: &str) -> Result<Value> {
        self.request(
            Method::DELETE,
            &format!("/v2/monitor/{monitor_id}"),
            None,
            &[],
        )
        .await
    }
    pub async fn run_monitor(&self, monitor_id: &str) -> Result<Value> {
        self.post(&format!("/v2/monitor/{monitor_id}/run"), json!({}))
            .await
    }
    pub async fn monitor_checks(&self, monitor_id: &str, check_id: Option<&str>) -> Result<Value> {
        self.get(
            &format!(
                "/v2/monitor/{monitor_id}/checks{}",
                check_id.map(|id| format!("/{id}")).unwrap_or_default()
            ),
            &[],
        )
        .await
    }

    pub async fn poll_crawl(
        &self,
        job_id: &str,
        offset: usize,
        limit: usize,
        interval: Duration,
        timeout: Duration,
    ) -> Result<Value> {
        self.poll(
            || self.crawl_status(job_id, offset, limit),
            job_id,
            interval,
            timeout,
        )
        .await
    }

    pub async fn poll_batch_scrape(
        &self,
        job_id: &str,
        offset: usize,
        limit: usize,
        interval: Duration,
        timeout: Duration,
    ) -> Result<Value> {
        self.poll(
            || self.batch_scrape_status(job_id, offset, limit),
            job_id,
            interval,
            timeout,
        )
        .await
    }

    async fn poll<F, Fut>(
        &self,
        mut status: F,
        job_id: &str,
        interval: Duration,
        timeout: Duration,
    ) -> Result<Value>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Value>>,
    {
        let deadline = Instant::now() + timeout;
        loop {
            let result = status().await?;
            if matches!(
                result.get("status").and_then(Value::as_str),
                Some("completed" | "failed" | "cancelled")
            ) {
                return Ok(result);
            }
            if Instant::now() >= deadline {
                return Err(BeeCrawlError::PollTimeout {
                    job_id: job_id.to_string(),
                });
            }
            tokio::time::sleep(interval).await;
        }
    }

    async fn post(&self, path: &str, payload: Value) -> Result<Value> {
        self.request(Method::POST, path, Some(payload), &[]).await
    }

    async fn get(&self, path: &str, params: &[(&str, String)]) -> Result<Value> {
        self.request(Method::GET, path, None, params).await
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        payload: Option<Value>,
        params: &[(&str, String)],
    ) -> Result<Value> {
        let mut request = self
            .http
            .request(method, format!("{}{}", self.base_url, path));
        if let Some(api_key) = &self.api_key {
            request = request.header("X-Web-Extract-Api-Key", api_key);
        }
        if !params.is_empty() {
            request = request.query(params);
        }
        if let Some(payload) = payload {
            request = request.json(&payload);
        }
        let response = request.send().await?;
        let status = response.status();
        let body = response.text().await?;
        let payload: Value = serde_json::from_str(&body)?;
        if !status.is_success() {
            let detail = payload.get("detail").cloned().unwrap_or(payload);
            let message = detail
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("BeeCrawl request failed")
                .to_string();
            return Err(BeeCrawlError::Api {
                status,
                message,
                detail,
            });
        }
        Ok(payload)
    }

    async fn request_bytes(&self, method: Method, path: &str, data: Vec<u8>) -> Result<Value> {
        let mut request = self
            .http
            .request(method, format!("{}{}", self.base_url, path))
            .body(data);
        if let Some(api_key) = &self.api_key {
            request = request.header("X-Web-Extract-Api-Key", api_key);
        }
        decode_response(request.send().await?).await
    }

    async fn request_form(&self, path: &str, form: reqwest::multipart::Form) -> Result<Value> {
        let mut request = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .multipart(form);
        if let Some(api_key) = &self.api_key {
            request = request.header("X-Web-Extract-Api-Key", api_key);
        }
        decode_response(request.send().await?).await
    }
}

async fn decode_response(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let body = response.text().await?;
    let payload: Value = serde_json::from_str(&body)?;
    if !status.is_success() {
        let detail = payload.get("detail").cloned().unwrap_or(payload);
        let message = detail
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("BeeCrawl request failed")
            .to_string();
        return Err(BeeCrawlError::Api {
            status,
            message,
            detail,
        });
    }
    Ok(payload)
}

#[derive(Clone, Debug)]
pub struct BeeCrawlClientBuilder {
    base_url: String,
    api_key: Option<String>,
    timeout: Duration,
}

impl BeeCrawlClientBuilder {
    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn build(self) -> Result<BeeCrawlClient> {
        let base_url = self.base_url.trim_end_matches('/').to_string();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(BeeCrawlError::InvalidBaseUrl(base_url));
        }
        Ok(BeeCrawlClient {
            http: HttpClient::builder().timeout(self.timeout).build()?,
            base_url,
            api_key: self.api_key,
        })
    }
}

fn merge_url(options: Value, url: &str) -> Value {
    merge_field(options, "url", url)
}

fn merge_field<T: serde::Serialize>(options: Value, field: &str, value: T) -> Value {
    let mut object = match options {
        Value::Object(object) => object,
        _ => Map::new(),
    };
    object.insert(field.to_string(), json!(value));
    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Request, State};
    use axum::routing::any;
    use axum::{Json, Router};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn v2_workflow_and_browser_methods_use_public_routes() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .fallback(any(
                |State(requests): State<Arc<Mutex<Vec<String>>>>, request: Request| async move {
                    requests.lock().unwrap().push(format!(
                        "{} {}",
                        request.method(),
                        request.uri().path()
                    ));
                    Json(json!({"success": true}))
                },
            ))
            .with_state(requests.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = BeeCrawlClient::new(format!("http://{address}"));

        client
            .v2_scrape("https://example.com", json!({}))
            .await
            .unwrap();
        client
            .create_agent("research", json!({"maxCredits": 2}))
            .await
            .unwrap();
        client
            .update_monitor("monitor", json!({"enabled": false}))
            .await
            .unwrap();
        client
            .browser_replay("session", Some("page"))
            .await
            .unwrap();

        assert_eq!(
            *requests.lock().unwrap(),
            [
                "POST /v2/scrape",
                "POST /v2/agent",
                "PATCH /v2/monitor/monitor",
                "GET /v2/browser/session/replay/page",
            ]
        );
    }
}
