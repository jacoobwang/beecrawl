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
