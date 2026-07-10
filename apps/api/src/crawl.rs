use std::time::Duration;

use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use thiserror::Error;
use tokio::time::sleep;
use uuid::Uuid;

use crate::models::{
    CrawlEnqueueResponse, CrawlError, CrawlRequest, CrawlStatusResponse, WebExtractMapRequest,
    WebExtractScrapeRequest, WebExtractScrapeResponse,
};
use crate::web_extract::{self, WebExtractError};

#[derive(Clone)]
pub struct CrawlStore {
    pool: Result<PgPool, String>,
}

#[derive(Debug, Error)]
pub enum CrawlStoreError {
    #[error("crawl_storage_unavailable: {0}")]
    StorageUnavailable(String),
    #[error("invalid_crawl_request: {0}")]
    InvalidRequest(String),
    #[error("crawl_storage_failed: {0}")]
    Database(#[from] sqlx::Error),
}

impl CrawlStoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::StorageUnavailable(_) => "crawl_storage_unavailable",
            Self::InvalidRequest(_) => "invalid_crawl_request",
            Self::Database(_) => "crawl_storage_failed",
        }
    }

    pub fn status(&self) -> axum::http::StatusCode {
        match self {
            Self::StorageUnavailable(_) => axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Self::InvalidRequest(_) => axum::http::StatusCode::BAD_REQUEST,
            Self::Database(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl CrawlStore {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            pool: Err(message.into()),
        }
    }

    pub fn from_env() -> Self {
        let pool = std::env::var("BEECRAWL_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .map_err(|_| "Set BEECRAWL_DATABASE_URL or DATABASE_URL to enable /crawl".to_string())
            .and_then(|url| {
                PgPoolOptions::new()
                    .max_connections(database_max_connections())
                    .connect_lazy(&url)
                    .map_err(|error| error.to_string())
            });
        Self { pool }
    }

    fn pool(&self) -> Result<&PgPool, CrawlStoreError> {
        self.pool
            .as_ref()
            .map_err(|error| CrawlStoreError::StorageUnavailable(error.clone()))
    }

    pub async fn enqueue(
        &self,
        request: CrawlRequest,
    ) -> Result<CrawlEnqueueResponse, CrawlStoreError> {
        if request.limit == 0 {
            return Err(CrawlStoreError::InvalidRequest(
                "limit must be greater than zero".to_string(),
            ));
        }
        let url = web_extract::normalize_url(&request.url)
            .map_err(|error| CrawlStoreError::InvalidRequest(error.to_string()))?;
        let id = Uuid::new_v4();
        let pool = self.pool()?;
        let mut transaction = pool.begin().await?;
        sqlx::query(
            "INSERT INTO crawl_jobs (id, url, status, page_limit, max_depth, include_subdomains, ignore_query_parameters, timeout_seconds, wait_for_ms, use_browser) VALUES ($1, $2, 'queued', $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(id)
        .bind(&url)
        .bind(request.limit as i64)
        .bind(request.max_depth as i32)
        .bind(request.include_subdomains)
        .bind(request.ignore_query_parameters)
        .bind(request.timeout_seconds as i64)
        .bind(request.wait_for_ms as i64)
        .bind(&request.use_browser)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("INSERT INTO crawl_tasks (id, crawl_id, url, depth, status) VALUES ($1, $2, $3, 0, 'queued')")
            .bind(Uuid::new_v4())
            .bind(id)
            .bind(&url)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(CrawlEnqueueResponse {
            id: id.to_string(),
            url,
            status: "queued".to_string(),
        })
    }

    pub async fn get(&self, id: &str) -> Result<Option<CrawlStatusResponse>, CrawlStoreError> {
        let id = parse_id(id)?;
        let pool = self.pool()?;
        let job = sqlx::query("SELECT id, url, status FROM crawl_jobs WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
        let Some(job) = job else { return Ok(None) };
        let counts = sqlx::query("SELECT COUNT(*) AS total, COUNT(*) FILTER (WHERE status = 'completed') AS completed, COUNT(*) FILTER (WHERE status = 'failed') AS failed FROM crawl_tasks WHERE crawl_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?;
        let rows = sqlx::query("SELECT result, error_code, error_message, url FROM crawl_tasks WHERE crawl_id = $1 AND (result IS NOT NULL OR status = 'failed') ORDER BY finished_at ASC NULLS LAST")
            .bind(id)
            .fetch_all(pool)
            .await?;
        let mut data = Vec::new();
        let mut errors = Vec::new();
        for row in rows {
            let url: String = row.try_get("url")?;
            if let Some(result) = row.try_get::<Option<Value>, _>("result")? {
                data.push(
                    serde_json::from_value::<WebExtractScrapeResponse>(result).map_err(
                        |error| CrawlStoreError::Database(sqlx::Error::Decode(Box::new(error))),
                    )?,
                );
            } else {
                errors.push(CrawlError {
                    url,
                    code: row
                        .try_get::<Option<String>, _>("error_code")?
                        .unwrap_or_else(|| "crawl_failed".to_string()),
                    message: row
                        .try_get::<Option<String>, _>("error_message")?
                        .unwrap_or_else(|| "Crawl task failed".to_string()),
                });
            }
        }
        Ok(Some(CrawlStatusResponse {
            id: job.try_get::<Uuid, _>("id")?.to_string(),
            url: job.try_get("url")?,
            status: job.try_get("status")?,
            total: count(&counts, "total")?,
            completed: count(&counts, "completed")?,
            failed: count(&counts, "failed")?,
            data,
            errors,
        }))
    }

    pub async fn cancel(&self, id: &str) -> Result<Option<CrawlStatusResponse>, CrawlStoreError> {
        let id = parse_id(id)?;
        let pool = self.pool()?;
        let updated = sqlx::query("UPDATE crawl_jobs SET cancel_requested = true, status = 'cancelled', finished_at = COALESCE(finished_at, now()) WHERE id = $1 AND status IN ('queued', 'scraping')")
            .bind(id)
            .execute(pool)
            .await?;
        if updated.rows_affected() == 0 {
            return self.get(&id.to_string()).await;
        }
        self.get(&id.to_string()).await
    }

    async fn claim_task(&self, worker_id: &str) -> Result<Option<ClaimedTask>, CrawlStoreError> {
        let pool = self.pool()?;
        let lease_token = Uuid::new_v4();
        let row = sqlx::query(
            "WITH candidate AS (SELECT tasks.id FROM crawl_tasks AS tasks JOIN crawl_jobs AS jobs ON jobs.id = tasks.crawl_id WHERE jobs.cancel_requested = false AND jobs.status IN ('queued', 'scraping') AND (tasks.status = 'queued' OR (tasks.status = 'active' AND tasks.lease_expires_at < now())) ORDER BY tasks.created_at FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE crawl_tasks AS tasks SET status = 'active', attempts = tasks.attempts + 1, lease_token = $1, lease_expires_at = now() + make_interval(secs => 90), worker_id = $2, started_at = COALESCE(tasks.started_at, now()) FROM candidate WHERE tasks.id = candidate.id RETURNING tasks.id, tasks.crawl_id, tasks.url, tasks.depth, tasks.lease_token",
        )
        .bind(lease_token)
        .bind(worker_id)
        .fetch_optional(pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let crawl_id: Uuid = row.try_get("crawl_id")?;
        sqlx::query("UPDATE crawl_jobs SET status = 'scraping', started_at = COALESCE(started_at, now()) WHERE id = $1 AND status = 'queued'")
            .bind(crawl_id)
            .execute(pool)
            .await?;
        Ok(Some(ClaimedTask {
            id: row.try_get("id")?,
            crawl_id,
            url: row.try_get("url")?,
            depth: row.try_get::<i32, _>("depth")? as usize,
            lease_token: row.try_get("lease_token")?,
        }))
    }

    async fn options(&self, crawl_id: Uuid) -> Result<CrawlOptions, CrawlStoreError> {
        let row = sqlx::query("SELECT page_limit, max_depth, include_subdomains, ignore_query_parameters, timeout_seconds, wait_for_ms, use_browser, cancel_requested FROM crawl_jobs WHERE id = $1")
            .bind(crawl_id)
            .fetch_one(self.pool()?)
            .await?;
        Ok(CrawlOptions {
            page_limit: row.try_get::<i64, _>("page_limit")? as usize,
            max_depth: row.try_get::<i32, _>("max_depth")? as usize,
            include_subdomains: row.try_get("include_subdomains")?,
            ignore_query_parameters: row.try_get("ignore_query_parameters")?,
            timeout_seconds: row.try_get::<i64, _>("timeout_seconds")? as u64,
            wait_for_ms: row.try_get::<i64, _>("wait_for_ms")? as u64,
            use_browser: row.try_get("use_browser")?,
            cancelled: row.try_get("cancel_requested")?,
        })
    }

    async fn finish_success(
        &self,
        task: &ClaimedTask,
        page: WebExtractScrapeResponse,
        links: Vec<String>,
    ) -> Result<(), CrawlStoreError> {
        let pool = self.pool()?;
        let mut transaction = pool.begin().await?;
        let job = sqlx::query("SELECT cancel_requested FROM crawl_jobs WHERE id = $1 FOR UPDATE")
            .bind(task.crawl_id)
            .fetch_one(&mut *transaction)
            .await?;
        let cancel_requested: bool = job.try_get("cancel_requested")?;
        let updated = sqlx::query("UPDATE crawl_tasks SET status = 'completed', result = $1, lease_expires_at = NULL, finished_at = now() WHERE id = $2 AND lease_token = $3 AND status = 'active'")
            .bind(serde_json::to_value(page).expect("scrape response serializes"))
            .bind(task.id)
            .bind(task.lease_token)
            .execute(&mut *transaction)
            .await?;
        if updated.rows_affected() == 0 {
            transaction.rollback().await?;
            return Ok(());
        }
        if !cancel_requested {
            for link in links {
                sqlx::query("INSERT INTO crawl_tasks (id, crawl_id, url, depth, status) SELECT $1, $2, $3, $4, 'queued' WHERE (SELECT COUNT(*) FROM crawl_tasks WHERE crawl_id = $2) < (SELECT page_limit FROM crawl_jobs WHERE id = $2) ON CONFLICT (crawl_id, url) DO NOTHING")
                    .bind(Uuid::new_v4())
                    .bind(task.crawl_id)
                    .bind(link)
                    .bind((task.depth + 1) as i32)
                    .execute(&mut *transaction)
                    .await?;
            }
        }
        complete_if_drained(&mut transaction, task.crawl_id).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn finish_failure(
        &self,
        task: &ClaimedTask,
        error: WebExtractError,
    ) -> Result<(), CrawlStoreError> {
        let pool = self.pool()?;
        let mut transaction = pool.begin().await?;
        sqlx::query("SELECT id FROM crawl_jobs WHERE id = $1 FOR UPDATE")
            .bind(task.crawl_id)
            .fetch_one(&mut *transaction)
            .await?;
        let updated = sqlx::query("UPDATE crawl_tasks SET status = 'failed', error_code = $1, error_message = $2, lease_expires_at = NULL, finished_at = now() WHERE id = $3 AND lease_token = $4 AND status = 'active'")
            .bind(error.code())
            .bind(error.to_string())
            .bind(task.id)
            .bind(task.lease_token)
            .execute(&mut *transaction)
            .await?;
        if updated.rows_affected() > 0 {
            complete_if_drained(&mut transaction, task.crawl_id).await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

pub async fn run_worker_forever(store: CrawlStore) -> anyhow::Result<()> {
    let worker_id = std::env::var("BEECRAWL_WORKER_ID")
        .unwrap_or_else(|_| format!("worker-{}", Uuid::new_v4()));
    let client = reqwest::Client::new();
    loop {
        match store.claim_task(&worker_id).await? {
            Some(task) => process_task(&store, &client, task).await?,
            None => sleep(Duration::from_millis(500)).await,
        }
    }
}

async fn process_task(
    store: &CrawlStore,
    client: &reqwest::Client,
    task: ClaimedTask,
) -> Result<(), CrawlStoreError> {
    let options = store.options(task.crawl_id).await?;
    if options.cancelled {
        return Ok(());
    }
    let page = web_extract::scrape(
        client,
        WebExtractScrapeRequest {
            url: task.url.clone(),
            formats: vec!["markdown".to_string()],
            location: None,
            timeout_seconds: options.timeout_seconds,
            wait_for_ms: options.wait_for_ms,
            use_browser: options.use_browser.clone(),
        },
    )
    .await;
    match page {
        Ok(page) => {
            let links = if task.depth < options.max_depth {
                web_extract::map_site(
                    client,
                    WebExtractMapRequest {
                        url: task.url.clone(),
                        search: None,
                        limit: options.page_limit,
                        include_subdomains: options.include_subdomains,
                        sitemap: if task.depth == 0 {
                            "include".to_string()
                        } else {
                            "skip".to_string()
                        },
                        ignore_sitemap: false,
                        ignore_query_parameters: options.ignore_query_parameters,
                    },
                )
                .await
                .map(|response| response.links)
                .unwrap_or_default()
            } else {
                vec![]
            };
            store.finish_success(&task, page, links).await
        }
        Err(error) => store.finish_failure(&task, error).await,
    }
}

struct ClaimedTask {
    id: Uuid,
    crawl_id: Uuid,
    url: String,
    depth: usize,
    lease_token: Uuid,
}

struct CrawlOptions {
    page_limit: usize,
    max_depth: usize,
    include_subdomains: bool,
    ignore_query_parameters: bool,
    timeout_seconds: u64,
    wait_for_ms: u64,
    use_browser: String,
    cancelled: bool,
}

async fn complete_if_drained(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    crawl_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE crawl_jobs SET status = 'completed', finished_at = now() WHERE id = $1 AND cancel_requested = false AND NOT EXISTS (SELECT 1 FROM crawl_tasks WHERE crawl_id = $1 AND status IN ('queued', 'active'))")
        .bind(crawl_id)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn parse_id(value: &str) -> Result<Uuid, CrawlStoreError> {
    Uuid::parse_str(value)
        .map_err(|_| CrawlStoreError::InvalidRequest("Invalid crawl ID".to_string()))
}

fn count(row: &sqlx::postgres::PgRow, column: &str) -> Result<usize, CrawlStoreError> {
    Ok(row.try_get::<i64, _>(column)? as usize)
}

fn database_max_connections() -> u32 {
    std::env::var("BEECRAWL_DATABASE_MAX_CONNECTIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10)
}
