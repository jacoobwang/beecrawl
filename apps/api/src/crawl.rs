use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::models::{
    CrawlEnqueueResponse, CrawlError, CrawlRequest, CrawlStatusResponse, WebExtractMapRequest,
    WebExtractScrapeRequest,
};
use crate::web_extract::{self, WebExtractError};

#[derive(Clone, Default)]
pub struct CrawlStore {
    jobs: Arc<Mutex<HashMap<String, CrawlJob>>>,
}

#[derive(Clone)]
struct CrawlJob {
    response: CrawlStatusResponse,
    cancelled: bool,
}

impl CrawlStore {
    pub async fn enqueue(
        &self,
        client: reqwest::Client,
        request: CrawlRequest,
    ) -> Result<CrawlEnqueueResponse, WebExtractError> {
        let url = web_extract::normalize_url(&request.url)?;
        let id = format!("crawl_{}", Uuid::new_v4().simple());
        let response = CrawlEnqueueResponse {
            id: id.clone(),
            url: url.clone(),
            status: "queued".to_string(),
        };
        self.jobs.lock().await.insert(
            id.clone(),
            CrawlJob {
                response: CrawlStatusResponse {
                    id: id.clone(),
                    url: url.clone(),
                    status: "queued".to_string(),
                    total: 0,
                    completed: 0,
                    failed: 0,
                    data: vec![],
                    errors: vec![],
                },
                cancelled: false,
            },
        );

        let store = self.clone();
        tokio::spawn(async move { store.run(id, url, request, client).await });
        Ok(response)
    }

    pub async fn get(&self, id: &str) -> Option<CrawlStatusResponse> {
        self.jobs
            .lock()
            .await
            .get(id)
            .map(|job| job.response.clone())
    }

    pub async fn cancel(&self, id: &str) -> Option<CrawlStatusResponse> {
        let mut jobs = self.jobs.lock().await;
        let job = jobs.get_mut(id)?;
        job.cancelled = true;
        if matches!(job.response.status.as_str(), "queued" | "scraping") {
            job.response.status = "cancelled".to_string();
        }
        Some(job.response.clone())
    }

    async fn run(
        &self,
        id: String,
        root_url: String,
        request: CrawlRequest,
        client: reqwest::Client,
    ) {
        self.set_status(&id, "scraping").await;
        let mut queue = VecDeque::from([(root_url, 0usize)]);
        let mut seen = HashSet::new();

        while let Some((url, depth)) = queue.pop_front() {
            if self.is_cancelled(&id).await || seen.len() >= request.limit {
                break;
            }
            if !seen.insert(url.clone()) {
                continue;
            }

            match web_extract::scrape(
                &client,
                WebExtractScrapeRequest {
                    url: url.clone(),
                    formats: vec!["markdown".to_string()],
                    location: None,
                    timeout_seconds: request.timeout_seconds,
                    wait_for_ms: request.wait_for_ms,
                    use_browser: request.use_browser.clone(),
                },
            )
            .await
            {
                Ok(page) => self.add_page(&id, page).await,
                Err(error) => self.add_error(&id, url.clone(), error).await,
            }

            if depth >= request.max_depth || self.is_cancelled(&id).await {
                continue;
            }
            let sitemap = if depth == 0 { "include" } else { "skip" };
            if let Ok(discovered) = web_extract::map_site(
                &client,
                WebExtractMapRequest {
                    url,
                    search: None,
                    limit: request.limit.saturating_sub(seen.len()),
                    include_subdomains: request.include_subdomains,
                    sitemap: sitemap.to_string(),
                    ignore_sitemap: false,
                    ignore_query_parameters: request.ignore_query_parameters,
                },
            )
            .await
            {
                for link in discovered.links {
                    if !seen.contains(&link) {
                        queue.push_back((link, depth + 1));
                    }
                }
            }
        }

        if !self.is_cancelled(&id).await {
            self.set_status(&id, "completed").await;
        }
    }

    async fn set_status(&self, id: &str, status: &str) {
        if let Some(job) = self.jobs.lock().await.get_mut(id) {
            if !job.cancelled {
                job.response.status = status.to_string();
            }
        }
    }

    async fn is_cancelled(&self, id: &str) -> bool {
        self.jobs
            .lock()
            .await
            .get(id)
            .map(|job| job.cancelled)
            .unwrap_or(true)
    }

    async fn add_page(&self, id: &str, page: crate::models::WebExtractScrapeResponse) {
        if let Some(job) = self.jobs.lock().await.get_mut(id) {
            job.response.total += 1;
            job.response.completed += 1;
            job.response.data.push(page);
        }
    }

    async fn add_error(&self, id: &str, url: String, error: WebExtractError) {
        if let Some(job) = self.jobs.lock().await.get_mut(id) {
            job.response.total += 1;
            job.response.failed += 1;
            job.response.errors.push(CrawlError {
                url,
                code: error.code().to_string(),
                message: error.to_string(),
            });
        }
    }
}
