use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use regex::Regex;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use thiserror::Error;
use tokio::time::sleep;
use uuid::Uuid;

use crate::cache::CacheStore;
use crate::models::{
    ActiveCrawl, ActiveCrawlsResponse, BatchScrapeEnqueueResponse, BatchScrapeRequest,
    CrawlEnqueueResponse, CrawlError, CrawlPagination, CrawlRequest, CrawlStatusQuery,
    CrawlStatusResponse, FirecrawlJobError, FirecrawlJobErrorsResponse, FirecrawlWebhook,
    ProxyConfig, WebExtractMapRequest, WebExtractScrapeRequest, WebExtractScrapeResponse,
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
    #[error("idempotency_key_already_used: {0}")]
    IdempotencyConflict(String),
    #[error("crawl_storage_failed: {0}")]
    Database(#[from] sqlx::Error),
}

impl CrawlStoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::StorageUnavailable(_) => "crawl_storage_unavailable",
            Self::InvalidRequest(_) => "invalid_crawl_request",
            Self::IdempotencyConflict(_) => "idempotency_key_already_used",
            Self::Database(_) => "crawl_storage_failed",
        }
    }

    pub fn status(&self) -> axum::http::StatusCode {
        match self {
            Self::StorageUnavailable(_) => axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Self::InvalidRequest(_) => axum::http::StatusCode::BAD_REQUEST,
            Self::IdempotencyConflict(_) => axum::http::StatusCode::CONFLICT,
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
        let url = std::env::var("BEECRAWL_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .map_err(|_| "Set BEECRAWL_DATABASE_URL or DATABASE_URL to enable /crawl".to_string());
        match url {
            Ok(url) => Self::from_database_url(&url),
            Err(error) => Self::unavailable(error),
        }
    }

    pub fn from_database_url(url: &str) -> Self {
        Self {
            pool: PgPoolOptions::new()
                .max_connections(database_max_connections())
                .connect_lazy(url)
                .map_err(|error| error.to_string()),
        }
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
        if request.max_concurrency == 0 {
            return Err(CrawlStoreError::InvalidRequest(
                "maxConcurrency must be greater than zero".to_string(),
            ));
        }
        let url = web_extract::normalize_url(&request.url)
            .map_err(|error| CrawlStoreError::InvalidRequest(error.to_string()))?;
        validate_path_patterns(&request.include_paths, "includePaths")?;
        validate_path_patterns(&request.exclude_paths, "excludePaths")?;
        let id = Uuid::new_v4();
        let pool = self.pool()?;
        let mut transaction = pool.begin().await?;
        if let Some(key) = request.idempotency_key.as_deref() {
            let key = Uuid::parse_str(key).map_err(|_| {
                CrawlStoreError::InvalidRequest("x-idempotency-key must be a UUID".to_string())
            })?;
            let inserted = sqlx::query(
                "INSERT INTO idempotency_keys (key) VALUES ($1) ON CONFLICT DO NOTHING",
            )
            .bind(key)
            .execute(&mut *transaction)
            .await?;
            if inserted.rows_affected() == 0 {
                transaction.rollback().await?;
                return Err(CrawlStoreError::IdempotencyConflict(key.to_string()));
            }
        }
        sqlx::query(
            "INSERT INTO crawl_jobs (id, url, status, page_limit, max_depth, include_paths, exclude_paths, regex_on_full_url, include_subdomains, allow_external_links, crawl_entire_domain, sitemap, delay_ms, max_concurrency, deduplicate_similar_urls, ignore_query_parameters, ignore_robots_txt, robots_user_agent, webhook, proxy, timeout_seconds, wait_for_ms, use_browser, skip_tls_verification, max_retries, expires_at) VALUES ($1, $2, 'queued', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, now() + make_interval(days => $25))",
        )
        .bind(id)
        .bind(&url)
        .bind(request.limit as i64)
        .bind(request.max_depth as i32)
        .bind(serde_json::to_value(&request.include_paths).expect("patterns serialize"))
        .bind(serde_json::to_value(&request.exclude_paths).expect("patterns serialize"))
        .bind(request.regex_on_full_url)
        .bind(request.include_subdomains)
        .bind(request.allow_external_links)
        .bind(request.crawl_entire_domain)
        .bind(&request.sitemap)
        .bind(request.delay_ms as i64)
        .bind(request.max_concurrency as i32)
        .bind(request.deduplicate_similar_urls)
        .bind(request.ignore_query_parameters)
        .bind(request.ignore_robots_txt)
        .bind(&request.robots_user_agent)
        .bind(
            request
                .webhook
                .as_ref()
                .map(|value| serde_json::to_value(value).expect("webhook serializes")),
        )
        .bind(
            request
                .proxy
                .as_ref()
                .map(|value| serde_json::to_value(value).expect("proxy serializes")),
        )
        .bind(request.timeout_seconds as i64)
        .bind(request.wait_for_ms as i64)
        .bind(&request.use_browser)
        .bind(request.skip_tls_verification)
        .bind(request.max_retries as i32)
        .bind(crawl_retention_days())
        .execute(&mut *transaction)
        .await?;
        sqlx::query("INSERT INTO crawl_tasks (id, crawl_id, url, dedup_key, depth, status) VALUES ($1, $2, $3, $4, 0, 'queued')")
            .bind(Uuid::new_v4())
            .bind(id)
            .bind(&url)
            .bind(crawl_dedup_key(&url, request.deduplicate_similar_urls))
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(CrawlEnqueueResponse {
            id: id.to_string(),
            url,
            status: "queued".to_string(),
        })
    }

    pub async fn enqueue_batch(
        &self,
        request: BatchScrapeRequest,
    ) -> Result<BatchScrapeEnqueueResponse, CrawlStoreError> {
        let mut urls = Vec::with_capacity(request.urls.len());
        for raw_url in request.urls {
            let url = web_extract::normalize_url(&raw_url)
                .map_err(|error| CrawlStoreError::InvalidRequest(error.to_string()))?;
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
        if urls.is_empty() {
            return Err(CrawlStoreError::InvalidRequest(
                "urls must contain at least one valid URL".to_string(),
            ));
        }
        if urls.len() > 1000 {
            return Err(CrawlStoreError::InvalidRequest(
                "batch scrape supports at most 1000 URLs".to_string(),
            ));
        }
        if request.max_concurrency == 0 {
            return Err(CrawlStoreError::InvalidRequest(
                "maxConcurrency must be greater than zero".to_string(),
            ));
        }

        let id = Uuid::new_v4();
        let pool = self.pool()?;
        let mut transaction = pool.begin().await?;
        sqlx::query(
            "INSERT INTO crawl_jobs (id, url, job_type, status, page_limit, max_depth, include_subdomains, ignore_query_parameters, max_concurrency, webhook, proxy, timeout_seconds, wait_for_ms, use_browser, skip_tls_verification, max_retries, expires_at) VALUES ($1, $2, 'batch_scrape', 'queued', $3, 0, false, true, $4, $5, $6, $7, $8, $9, $10, $11, now() + make_interval(days => $12))",
        )
        .bind(id)
        .bind(&urls[0])
        .bind(urls.len() as i64)
        .bind(request.max_concurrency as i32)
        .bind(
            request
                .webhook
                .as_ref()
                .map(|value| serde_json::to_value(value).expect("webhook serializes")),
        )
        .bind(
            request
                .proxy
                .as_ref()
                .map(|value| serde_json::to_value(value).expect("proxy serializes")),
        )
        .bind(request.timeout_seconds as i64)
        .bind(request.wait_for_ms as i64)
        .bind(&request.use_browser)
        .bind(request.skip_tls_verification)
        .bind(request.max_retries as i32)
        .bind(crawl_retention_days())
        .execute(&mut *transaction)
        .await?;
        for url in &urls {
            sqlx::query("INSERT INTO crawl_tasks (id, crawl_id, url, dedup_key, depth, status) VALUES ($1, $2, $3, $3, 0, 'queued')")
                .bind(Uuid::new_v4())
                .bind(id)
                .bind(url)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(BatchScrapeEnqueueResponse {
            id: id.to_string(),
            status: "queued".to_string(),
            total: urls.len(),
        })
    }

    pub async fn get(
        &self,
        id: &str,
        query: CrawlStatusQuery,
    ) -> Result<Option<CrawlStatusResponse>, CrawlStoreError> {
        let id = parse_id(id)?;
        let pool = self.pool()?;
        let job = sqlx::query("SELECT id, url, status, expires_at FROM crawl_jobs WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
        let Some(job) = job else { return Ok(None) };
        let counts = sqlx::query("SELECT COUNT(*) AS total, COUNT(*) FILTER (WHERE status = 'completed') AS completed, COUNT(*) FILTER (WHERE status = 'failed') AS failed FROM crawl_tasks WHERE crawl_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?;
        let limit = query.limit.clamp(1, 100);
        let results_total = count(&counts, "completed")? + count(&counts, "failed")?;
        let rows = sqlx::query("SELECT result, error_code, error_message, url FROM crawl_tasks WHERE crawl_id = $1 AND (result IS NOT NULL OR status = 'failed') ORDER BY finished_at ASC NULLS LAST OFFSET $2 LIMIT $3")
            .bind(id)
            .bind(query.offset as i64)
            .bind(limit as i64)
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
            pagination: CrawlPagination {
                offset: query.offset,
                limit,
                total: results_total,
                next: (query.offset + limit < results_total).then_some(query.offset + limit),
            },
            expires_at: job
                .try_get::<Option<DateTime<Utc>>, _>("expires_at")?
                .map(|value| value.to_rfc3339()),
        }))
    }

    pub async fn errors(
        &self,
        id: &str,
    ) -> Result<Option<FirecrawlJobErrorsResponse>, CrawlStoreError> {
        let id = parse_id(id)?;
        let pool = self.pool()?;
        let exists =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM crawl_jobs WHERE id = $1)")
                .bind(id)
                .fetch_one(pool)
                .await?;
        if !exists {
            return Ok(None);
        }
        let rows = sqlx::query("SELECT id, url, error_message, finished_at FROM crawl_tasks WHERE crawl_id = $1 AND status = 'failed' ORDER BY finished_at ASC NULLS LAST")
            .bind(id)
            .fetch_all(pool)
            .await?;
        let errors = rows
            .into_iter()
            .map(|row| {
                Ok(FirecrawlJobError {
                    id: row.try_get::<Uuid, _>("id")?.to_string(),
                    timestamp: row
                        .try_get::<Option<DateTime<Utc>>, _>("finished_at")?
                        .map(|value| value.to_rfc3339()),
                    url: row.try_get("url")?,
                    error: row
                        .try_get::<Option<String>, _>("error_message")?
                        .unwrap_or_else(|| "Crawl task failed".to_string()),
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Ok(Some(FirecrawlJobErrorsResponse {
            errors,
            robots_blocked: Vec::new(),
        }))
    }

    pub async fn active(&self) -> Result<ActiveCrawlsResponse, CrawlStoreError> {
        let rows = sqlx::query("SELECT id, url, status, job_type, page_limit, max_depth, include_subdomains, ignore_query_parameters, delay_ms, max_concurrency FROM crawl_jobs WHERE job_type = 'crawl' AND status IN ('queued', 'scraping') AND cancel_requested = false ORDER BY created_at ASC")
            .fetch_all(self.pool()?)
            .await?;
        let crawls = rows
            .into_iter()
            .map(|row| {
                let job_type: String = row.try_get("job_type")?;
                Ok(ActiveCrawl {
                    id: row.try_get::<Uuid, _>("id")?.to_string(),
                    team_id: "self-hosted".to_string(),
                    url: row.try_get("url")?,
                    status: row.try_get("status")?,
                    options: serde_json::json!({
                        "jobType": job_type,
                        "limit": row.try_get::<i64, _>("page_limit")?,
                        "maxDiscoveryDepth": row.try_get::<i32, _>("max_depth")?,
                        "allowSubdomains": row.try_get::<bool, _>("include_subdomains")?,
                        "ignoreQueryParameters": row.try_get::<bool, _>("ignore_query_parameters")?,
                        "delay": row.try_get::<i64, _>("delay_ms")? as f64 / 1000.0,
                        "maxConcurrency": row.try_get::<i32, _>("max_concurrency")?,
                    }),
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Ok(ActiveCrawlsResponse {
            success: true,
            crawls,
        })
    }

    pub async fn cancel(&self, id: &str) -> Result<Option<CrawlStatusResponse>, CrawlStoreError> {
        let id = parse_id(id)?;
        let pool = self.pool()?;
        let updated = sqlx::query("UPDATE crawl_jobs SET cancel_requested = true, status = 'cancelled', finished_at = COALESCE(finished_at, now()) WHERE id = $1 AND status IN ('queued', 'scraping')")
            .bind(id)
            .execute(pool)
            .await?;
        if updated.rows_affected() == 0 {
            return self.get(&id.to_string(), CrawlStatusQuery::default()).await;
        }
        self.get(&id.to_string(), CrawlStatusQuery::default()).await
    }

    async fn claim_task(&self, worker_id: &str) -> Result<Option<ClaimedTask>, CrawlStoreError> {
        let pool = self.pool()?;
        let lease_token = Uuid::new_v4();
        let row = sqlx::query(
            "WITH candidate AS (SELECT tasks.id, tasks.crawl_id FROM crawl_tasks AS tasks JOIN crawl_jobs AS jobs ON jobs.id = tasks.crawl_id WHERE jobs.cancel_requested = false AND jobs.status IN ('queued', 'scraping') AND ((tasks.status = 'queued' AND tasks.next_attempt_at <= now()) OR (tasks.status = 'active' AND tasks.lease_expires_at < now())) AND (jobs.last_task_started_at IS NULL OR jobs.last_task_started_at + make_interval(secs => jobs.delay_ms::double precision / 1000.0) <= now()) AND (SELECT COUNT(*) FROM crawl_tasks AS active WHERE active.crawl_id = jobs.id AND active.status = 'active' AND active.lease_expires_at >= now()) < jobs.max_concurrency ORDER BY tasks.next_attempt_at, tasks.created_at FOR UPDATE OF jobs, tasks SKIP LOCKED LIMIT 1), activated AS (UPDATE crawl_tasks AS tasks SET status = 'active', attempts = tasks.attempts + 1, lease_token = $1, lease_expires_at = now() + make_interval(secs => 90), worker_id = $2, started_at = COALESCE(tasks.started_at, now()) FROM candidate WHERE tasks.id = candidate.id RETURNING tasks.id, tasks.crawl_id, tasks.url, tasks.depth, tasks.attempts, tasks.lease_token), touched AS (UPDATE crawl_jobs AS jobs SET status = 'scraping', started_at = COALESCE(started_at, now()), last_task_started_at = now() FROM activated WHERE jobs.id = activated.crawl_id RETURNING jobs.id) SELECT activated.* FROM activated JOIN touched ON touched.id = activated.crawl_id",
        )
        .bind(lease_token)
        .bind(worker_id)
        .fetch_optional(pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let crawl_id: Uuid = row.try_get("crawl_id")?;
        Ok(Some(ClaimedTask {
            id: row.try_get("id")?,
            crawl_id,
            url: row.try_get("url")?,
            depth: row.try_get::<i32, _>("depth")? as usize,
            attempts: row.try_get("attempts")?,
            lease_token: row.try_get("lease_token")?,
        }))
    }

    async fn options(&self, crawl_id: Uuid) -> Result<CrawlOptions, CrawlStoreError> {
        let row = sqlx::query("SELECT job_type, page_limit, max_depth, include_paths, exclude_paths, regex_on_full_url, include_subdomains, allow_external_links, crawl_entire_domain, sitemap, ignore_query_parameters, ignore_robots_txt, robots_user_agent, proxy, timeout_seconds, wait_for_ms, use_browser, skip_tls_verification, cancel_requested FROM crawl_jobs WHERE id = $1")
            .bind(crawl_id)
            .fetch_one(self.pool()?)
        .await?;
        Ok(CrawlOptions {
            job_type: row.try_get("job_type")?,
            page_limit: row.try_get::<i64, _>("page_limit")? as usize,
            max_depth: row.try_get::<i32, _>("max_depth")? as usize,
            include_paths: serde_json::from_value(row.try_get("include_paths")?)
                .map_err(|error| CrawlStoreError::Database(sqlx::Error::Decode(Box::new(error))))?,
            exclude_paths: serde_json::from_value(row.try_get("exclude_paths")?)
                .map_err(|error| CrawlStoreError::Database(sqlx::Error::Decode(Box::new(error))))?,
            regex_on_full_url: row.try_get("regex_on_full_url")?,
            include_subdomains: row.try_get("include_subdomains")?,
            allow_external_links: row.try_get("allow_external_links")?,
            crawl_entire_domain: row.try_get("crawl_entire_domain")?,
            sitemap: row.try_get("sitemap")?,
            ignore_query_parameters: row.try_get("ignore_query_parameters")?,
            ignore_robots_txt: row.try_get("ignore_robots_txt")?,
            robots_user_agent: row.try_get("robots_user_agent")?,
            proxy: row
                .try_get::<Option<Value>, _>("proxy")?
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| CrawlStoreError::Database(sqlx::Error::Decode(Box::new(error))))?,
            timeout_seconds: row.try_get::<i64, _>("timeout_seconds")? as u64,
            wait_for_ms: row.try_get::<i64, _>("wait_for_ms")? as u64,
            use_browser: row.try_get("use_browser")?,
            skip_tls_verification: row.try_get("skip_tls_verification")?,
            cancelled: row.try_get("cancel_requested")?,
        })
    }

    async fn claim_job_webhook(
        &self,
        crawl_id: Uuid,
        event: &str,
    ) -> Result<Option<JobWebhook>, CrawlStoreError> {
        let query = match event {
            "started" => {
                "UPDATE crawl_jobs SET webhook_started_at = now() WHERE id = $1 AND webhook IS NOT NULL AND webhook_started_at IS NULL RETURNING job_type, webhook"
            }
            "completed" => {
                "UPDATE crawl_jobs SET webhook_completed_at = now() WHERE id = $1 AND webhook IS NOT NULL AND webhook_completed_at IS NULL AND status = 'completed' RETURNING job_type, webhook"
            }
            _ => return Ok(None),
        };
        let row = sqlx::query(query)
            .bind(crawl_id)
            .fetch_optional(self.pool()?)
            .await?;
        decode_job_webhook(row)
    }

    async fn claim_page_webhook(
        &self,
        task: &ClaimedTask,
    ) -> Result<Option<JobWebhook>, CrawlStoreError> {
        let row = sqlx::query("UPDATE crawl_tasks AS tasks SET webhook_delivered_at = now() FROM crawl_jobs AS jobs WHERE tasks.id = $1 AND tasks.crawl_id = jobs.id AND jobs.webhook IS NOT NULL AND tasks.webhook_delivered_at IS NULL AND tasks.status IN ('completed', 'failed') RETURNING jobs.job_type, jobs.webhook")
            .bind(task.id)
            .fetch_optional(self.pool()?)
            .await?;
        decode_job_webhook(row)
    }

    async fn finish_success(
        &self,
        task: &ClaimedTask,
        page: WebExtractScrapeResponse,
        links: Vec<String>,
    ) -> Result<(), CrawlStoreError> {
        let pool = self.pool()?;
        let mut transaction = pool.begin().await?;
        let job = sqlx::query("SELECT cancel_requested, deduplicate_similar_urls FROM crawl_jobs WHERE id = $1 FOR UPDATE")
            .bind(task.crawl_id)
            .fetch_one(&mut *transaction)
            .await?;
        let cancel_requested: bool = job.try_get("cancel_requested")?;
        let deduplicate_similar_urls: bool = job.try_get("deduplicate_similar_urls")?;
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
                let dedup_key = crawl_dedup_key(&link, deduplicate_similar_urls);
                sqlx::query("INSERT INTO crawl_tasks (id, crawl_id, url, dedup_key, depth, status) SELECT $1, $2, $3, $4, $5, 'queued' WHERE (SELECT COUNT(*) FROM crawl_tasks WHERE crawl_id = $2) < (SELECT page_limit FROM crawl_jobs WHERE id = $2) ON CONFLICT DO NOTHING")
                    .bind(Uuid::new_v4())
                    .bind(task.crawl_id)
                    .bind(link)
                    .bind(dedup_key)
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
        let job = sqlx::query(
            "SELECT cancel_requested, max_retries FROM crawl_jobs WHERE id = $1 FOR UPDATE",
        )
        .bind(task.crawl_id)
        .fetch_one(&mut *transaction)
        .await?;
        let cancelled: bool = job.try_get("cancel_requested")?;
        let max_retries: i32 = job.try_get("max_retries")?;
        let retryable = is_retryable(&error);
        let updated = sqlx::query("UPDATE crawl_tasks SET status = CASE WHEN $1 AND NOT $2 AND attempts <= $3 THEN 'queued' ELSE 'failed' END, error_code = $4, error_message = $5, lease_expires_at = NULL, next_attempt_at = CASE WHEN $1 AND NOT $2 AND attempts <= $3 THEN now() + make_interval(secs => $6) ELSE next_attempt_at END, finished_at = CASE WHEN $1 AND NOT $2 AND attempts <= $3 THEN NULL ELSE now() END WHERE id = $7 AND lease_token = $8 AND status = 'active' RETURNING attempts, status")
            .bind(retryable)
            .bind(cancelled)
            .bind(max_retries)
            .bind(error.code())
            .bind(error.to_string())
            .bind(retry_delay_seconds(task.attempts))
            .bind(task.id)
            .bind(task.lease_token)
            .fetch_optional(&mut *transaction)
            .await?;
        if updated.is_some() {
            complete_if_drained(&mut transaction, task.crawl_id).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn cleanup_expired(&self) -> Result<u64, CrawlStoreError> {
        let result = sqlx::query(
            "DELETE FROM crawl_jobs WHERE expires_at IS NOT NULL AND expires_at <= now() AND status IN ('completed', 'cancelled')",
        )
        .execute(self.pool()?)
        .await?;
        Ok(result.rows_affected())
    }
}

pub async fn run_worker_forever(store: CrawlStore) -> anyhow::Result<()> {
    let worker_id = std::env::var("BEECRAWL_WORKER_ID")
        .unwrap_or_else(|_| format!("worker-{}", Uuid::new_v4()));
    let client = reqwest::Client::new();
    let cache = CacheStore::from_env();
    let cleanup_interval = Duration::from_secs(crawl_cleanup_interval_seconds());
    let mut next_cleanup = Instant::now();
    loop {
        if Instant::now() >= next_cleanup {
            store.cleanup_expired().await?;
            cache.cleanup_expired().await;
            next_cleanup = Instant::now() + cleanup_interval;
        }
        match store.claim_task(&worker_id).await? {
            Some(task) => process_task(&store, &cache, &client, task).await?,
            None => sleep(Duration::from_millis(500)).await,
        }
    }
}

async fn process_task(
    store: &CrawlStore,
    cache: &CacheStore,
    client: &reqwest::Client,
    task: ClaimedTask,
) -> Result<(), CrawlStoreError> {
    let options = store.options(task.crawl_id).await?;
    if options.cancelled {
        return Ok(());
    }
    if let Some(config) = store.claim_job_webhook(task.crawl_id, "started").await? {
        let _ = crate::webhook::deliver(
            client,
            &config.webhook,
            &config.job_type,
            task.crawl_id,
            "started",
            true,
            json!([]),
            None,
        )
        .await;
    }
    let policy_error = if !url_allowed_by_patterns(
        &task.url,
        &options.include_paths,
        &options.exclude_paths,
        options.regex_on_full_url,
    ) {
        Some(WebExtractError::BlockedByPolicy(
            "URL is excluded by includePaths or excludePaths".to_string(),
        ))
    } else if !options.ignore_robots_txt
        && !web_extract::robots_allows(
            client,
            &task.url,
            options.robots_user_agent.as_deref(),
            options.timeout_seconds,
        )
        .await
    {
        Some(WebExtractError::BlockedByPolicy(
            "robots.txt disallows URL".to_string(),
        ))
    } else {
        None
    };
    let page = if let Some(error) = policy_error {
        Err(error)
    } else {
        web_extract::scrape_with_cache(
            client,
            cache,
            WebExtractScrapeRequest {
                url: task.url.clone(),
                formats: vec!["markdown".to_string()],
                location: None,
                timeout_seconds: options.timeout_seconds,
                wait_for_ms: options.wait_for_ms,
                use_browser: options.use_browser.clone(),
                skip_tls_verification: options.skip_tls_verification,
                headers: std::collections::HashMap::new(),
                proxy: options.proxy.clone(),
                screenshot: None,
                content: None,
            },
        )
        .await
    };
    match page {
        Ok(page) => {
            let webhook_document = webhook_document(&page);
            let links = if options.job_type == "crawl"
                && task.depth < options.max_depth
                && !(options.sitemap == "only" && task.depth > 0)
            {
                web_extract::map_site(
                    client,
                    WebExtractMapRequest {
                        url: task.url.clone(),
                        search: None,
                        limit: options.page_limit,
                        include_subdomains: options.include_subdomains,
                        allow_external_links: options.allow_external_links,
                        crawl_entire_domain: options.crawl_entire_domain,
                        sitemap: if task.depth == 0 {
                            options.sitemap.clone()
                        } else {
                            "skip".to_string()
                        },
                        ignore_sitemap: false,
                        ignore_query_parameters: options.ignore_query_parameters,
                    },
                )
                .await
                .map(|response| {
                    response
                        .links
                        .into_iter()
                        .filter(|url| {
                            url_allowed_by_patterns(
                                url,
                                &options.include_paths,
                                &options.exclude_paths,
                                options.regex_on_full_url,
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
            } else {
                vec![]
            };
            store.finish_success(&task, page, links).await?;
            if let Some(config) = store.claim_page_webhook(&task).await? {
                let _ = crate::webhook::deliver(
                    client,
                    &config.webhook,
                    &config.job_type,
                    task.crawl_id,
                    "page",
                    true,
                    json!([webhook_document]),
                    None,
                )
                .await;
            }
        }
        Err(error) => {
            let message = error.to_string();
            store.finish_failure(&task, error).await?;
            if let Some(config) = store.claim_page_webhook(&task).await? {
                let _ = crate::webhook::deliver(
                    client,
                    &config.webhook,
                    &config.job_type,
                    task.crawl_id,
                    "page",
                    false,
                    json!([]),
                    Some(&message),
                )
                .await;
            }
        }
    }
    if let Some(config) = store.claim_job_webhook(task.crawl_id, "completed").await? {
        let _ = crate::webhook::deliver(
            client,
            &config.webhook,
            &config.job_type,
            task.crawl_id,
            "completed",
            true,
            json!([]),
            None,
        )
        .await;
    }
    Ok(())
}

struct ClaimedTask {
    id: Uuid,
    crawl_id: Uuid,
    url: String,
    depth: usize,
    attempts: i32,
    lease_token: Uuid,
}

struct CrawlOptions {
    job_type: String,
    page_limit: usize,
    max_depth: usize,
    include_paths: Vec<String>,
    exclude_paths: Vec<String>,
    regex_on_full_url: bool,
    include_subdomains: bool,
    allow_external_links: bool,
    crawl_entire_domain: bool,
    sitemap: String,
    ignore_query_parameters: bool,
    ignore_robots_txt: bool,
    robots_user_agent: Option<String>,
    proxy: Option<ProxyConfig>,
    timeout_seconds: u64,
    wait_for_ms: u64,
    use_browser: String,
    skip_tls_verification: bool,
    cancelled: bool,
}

struct JobWebhook {
    job_type: String,
    webhook: FirecrawlWebhook,
}

fn decode_job_webhook(
    row: Option<sqlx::postgres::PgRow>,
) -> Result<Option<JobWebhook>, CrawlStoreError> {
    row.map(|row| {
        let value: Value = row.try_get("webhook")?;
        let webhook = serde_json::from_value(value)
            .map_err(|error| CrawlStoreError::Database(sqlx::Error::Decode(Box::new(error))))?;
        Ok(JobWebhook {
            job_type: row.try_get("job_type")?,
            webhook,
        })
    })
    .transpose()
}

fn webhook_document(page: &WebExtractScrapeResponse) -> Value {
    json!({
        "markdown": page.markdown,
        "html": page.html,
        "rawHtml": page.raw_html,
        "links": page.links,
        "screenshot": page.screenshot,
        "metadata": {
            "title": page.metadata.title,
            "language": page.metadata.language,
            "sourceURL": page.url,
            "url": page.final_url,
            "statusCode": page.metadata.status_code,
            "scrapeId": page.request_id,
        }
    })
}

fn validate_path_patterns(patterns: &[String], field: &str) -> Result<(), CrawlStoreError> {
    for pattern in patterns {
        Regex::new(pattern).map_err(|error| {
            CrawlStoreError::InvalidRequest(format!("Invalid {field} regex '{pattern}': {error}"))
        })?;
    }
    Ok(())
}

fn url_allowed_by_patterns(
    raw_url: &str,
    include_paths: &[String],
    exclude_paths: &[String],
    regex_on_full_url: bool,
) -> bool {
    let Ok(url) = url::Url::parse(raw_url) else {
        return false;
    };
    let target = if regex_on_full_url {
        raw_url
    } else {
        url.path()
    };
    let matches = |pattern: &String| Regex::new(pattern).is_ok_and(|regex| regex.is_match(target));
    !exclude_paths.iter().any(matches)
        && (include_paths.is_empty() || include_paths.iter().any(matches))
}

fn crawl_dedup_key(raw_url: &str, deduplicate_similar_urls: bool) -> String {
    if !deduplicate_similar_urls {
        return raw_url.to_string();
    }
    let Ok(mut url) = url::Url::parse(raw_url) else {
        return raw_url.to_string();
    };
    if let Some(host) = url.host_str().map(str::to_string) {
        let normalized = host.strip_prefix("www.").unwrap_or(&host);
        let _ = url.set_host(Some(normalized));
    }
    let mut path = url.path().to_string();
    for suffix in ["/index.html", "/index.php"] {
        if path.ends_with(suffix) {
            path.truncate(path.len() - suffix.len());
            path.push('/');
            break;
        }
    }
    if path.len() > 1 {
        path = path.trim_end_matches('/').to_string();
    }
    url.set_path(&path);
    url.set_scheme("http").ok();
    url.to_string()
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

fn crawl_retention_days() -> i32 {
    std::env::var("BEECRAWL_CRAWL_RETENTION_DAYS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &i32| *value > 0)
        .unwrap_or(7)
}

fn crawl_cleanup_interval_seconds() -> u64 {
    std::env::var("BEECRAWL_CRAWL_CLEANUP_INTERVAL_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &u64| *value > 0)
        .unwrap_or(3600)
}

fn retry_delay_seconds(attempts: i32) -> i32 {
    let exponent = attempts.saturating_sub(1).min(6) as u32;
    5_i32.saturating_mul(2_i32.pow(exponent)).min(300)
}

fn is_retryable(error: &WebExtractError) -> bool {
    matches!(
        error,
        WebExtractError::FetchFailed(_)
            | WebExtractError::RenderFailed(_)
            | WebExtractError::EmptyContent
    )
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use tokio::sync::Mutex;

    use super::*;
    use crate::models::{WebExtractMetadata, WebExtractScrapeResponse};

    static DATABASE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn store() -> Option<CrawlStore> {
        std::env::var("BEECRAWL_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.trim().is_empty())
            .map(|url| CrawlStore::from_database_url(&url))
    }

    fn request(max_retries: usize) -> CrawlRequest {
        CrawlRequest {
            url: "https://example.com".to_string(),
            idempotency_key: None,
            webhook: None,
            proxy: None,
            limit: 10,
            max_depth: 0,
            include_paths: Vec::new(),
            exclude_paths: Vec::new(),
            regex_on_full_url: false,
            include_subdomains: false,
            allow_external_links: false,
            crawl_entire_domain: false,
            sitemap: "include".to_string(),
            delay_ms: 0,
            max_concurrency: 10,
            deduplicate_similar_urls: true,
            ignore_query_parameters: true,
            ignore_robots_txt: false,
            robots_user_agent: None,
            timeout_seconds: 5,
            wait_for_ms: 0,
            use_browser: "never".to_string(),
            skip_tls_verification: false,
            max_retries,
        }
    }

    fn page(url: &str) -> WebExtractScrapeResponse {
        WebExtractScrapeResponse {
            request_id: Uuid::new_v4().to_string(),
            url: url.to_string(),
            final_url: url.to_string(),
            markdown: "# Example".to_string(),
            html: None,
            raw_html: None,
            links: None,
            screenshot: None,
            metadata: WebExtractMetadata {
                title: Some("Example".to_string()),
                language: Some("en".to_string()),
                status_code: Some(200),
                provider: "test".to_string(),
                rendered: false,
                elapsed_ms: Some(1),
            },
        }
    }

    async fn reset(store: &CrawlStore) {
        sqlx::query("TRUNCATE crawl_tasks, crawl_jobs, idempotency_keys CASCADE")
            .execute(store.pool().unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn postgres_status_paginates_completed_results() {
        let Some(store) = store() else { return };
        let _guard = DATABASE_LOCK.get_or_init(|| Mutex::new(())).lock().await;
        reset(&store).await;
        let crawl = store.enqueue(request(0)).await.unwrap();
        let crawl_id = Uuid::parse_str(&crawl.id).unwrap();
        let first = store.claim_task("test-worker").await.unwrap().unwrap();
        store
            .finish_success(&first, page("https://example.com"), vec![])
            .await
            .unwrap();
        for suffix in ["one", "two"] {
            let url = format!("https://example.com/{suffix}");
            sqlx::query("INSERT INTO crawl_tasks (id, crawl_id, url, dedup_key, depth, status, result, finished_at) VALUES ($1, $2, $3, $3, 1, 'completed', $4, now())")
                .bind(Uuid::new_v4())
                .bind(crawl_id)
                .bind(&url)
                .bind(serde_json::to_value(page(&url)).unwrap())
                .execute(store.pool().unwrap())
                .await
                .unwrap();
        }

        let first_page = store
            .get(
                &crawl.id,
                CrawlStatusQuery {
                    offset: 0,
                    limit: 2,
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first_page.completed, 3);
        assert_eq!(first_page.data.len(), 2);
        assert_eq!(first_page.pagination.total, 3);
        assert_eq!(first_page.pagination.next, Some(2));

        let second_page = store
            .get(
                &crawl.id,
                CrawlStatusQuery {
                    offset: 2,
                    limit: 2,
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second_page.data.len(), 1);
        assert_eq!(second_page.pagination.next, None);
        reset(&store).await;
    }

    #[tokio::test]
    async fn postgres_batch_scrape_enqueues_unique_urls_without_crawl_expansion() {
        let Some(store) = store() else { return };
        let _guard = DATABASE_LOCK.get_or_init(|| Mutex::new(())).lock().await;
        reset(&store).await;
        let batch = store
            .enqueue_batch(BatchScrapeRequest {
                urls: vec![
                    "https://example.com".to_string(),
                    "https://example.com".to_string(),
                    "https://example.com/docs".to_string(),
                ],
                max_concurrency: 3,
                webhook: None,
                proxy: None,
                timeout_seconds: 5,
                wait_for_ms: 0,
                use_browser: "never".to_string(),
                skip_tls_verification: false,
                max_retries: 0,
            })
            .await
            .unwrap();
        assert_eq!(batch.total, 2);
        let status = store
            .get(&batch.id, CrawlStatusQuery::default())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.status, "queued");
        assert_eq!(status.total, 2);
        assert_eq!(status.pagination.total, 0);
        let row = sqlx::query("SELECT job_type, max_depth FROM crawl_jobs WHERE id = $1")
            .bind(Uuid::parse_str(&batch.id).unwrap())
            .fetch_one(store.pool().unwrap())
            .await
            .unwrap();
        assert_eq!(
            row.try_get::<String, _>("job_type").unwrap(),
            "batch_scrape"
        );
        assert_eq!(row.try_get::<i32, _>("max_depth").unwrap(), 0);
        reset(&store).await;
    }

    #[tokio::test]
    async fn postgres_enforces_per_job_concurrency_when_claiming() {
        let Some(store) = store() else { return };
        let _guard = DATABASE_LOCK.get_or_init(|| Mutex::new(())).lock().await;
        reset(&store).await;
        let batch = store
            .enqueue_batch(BatchScrapeRequest {
                urls: vec![
                    "https://example.com/one".to_string(),
                    "https://example.com/two".to_string(),
                ],
                max_concurrency: 1,
                webhook: None,
                proxy: None,
                timeout_seconds: 5,
                wait_for_ms: 0,
                use_browser: "never".to_string(),
                skip_tls_verification: false,
                max_retries: 0,
            })
            .await
            .unwrap();
        let first = store.claim_task("worker-one").await.unwrap().unwrap();
        assert!(store.claim_task("worker-two").await.unwrap().is_none());
        let first_url = first.url.clone();
        store
            .finish_success(&first, page(&first_url), vec![])
            .await
            .unwrap();
        assert!(store.claim_task("worker-two").await.unwrap().is_some());
        store.cancel(&batch.id).await.unwrap();
        reset(&store).await;
    }

    #[tokio::test]
    async fn postgres_enforces_crawl_delay_when_claiming() {
        let Some(store) = store() else { return };
        let _guard = DATABASE_LOCK.get_or_init(|| Mutex::new(())).lock().await;
        reset(&store).await;
        let mut crawl_request = request(0);
        crawl_request.delay_ms = 60_000;
        crawl_request.max_depth = 1;
        let crawl = store.enqueue(crawl_request).await.unwrap();
        let first = store.claim_task("worker-one").await.unwrap().unwrap();
        store
            .finish_success(
                &first,
                page("https://example.com"),
                vec!["https://example.com/next".to_string()],
            )
            .await
            .unwrap();
        assert!(store.claim_task("worker-two").await.unwrap().is_none());
        sqlx::query(
            "UPDATE crawl_jobs SET last_task_started_at = now() - interval '61 seconds' WHERE id = $1",
        )
        .bind(Uuid::parse_str(&crawl.id).unwrap())
        .execute(store.pool().unwrap())
        .await
        .unwrap();
        assert!(store.claim_task("worker-two").await.unwrap().is_some());
        store.cancel(&crawl.id).await.unwrap();
        reset(&store).await;
    }

    #[tokio::test]
    async fn postgres_rejects_a_reused_idempotency_key() {
        let Some(store) = store() else { return };
        let _guard = DATABASE_LOCK.get_or_init(|| Mutex::new(())).lock().await;
        reset(&store).await;
        let key = Uuid::new_v4().to_string();
        let mut first = request(0);
        first.idempotency_key = Some(key.clone());
        store.enqueue(first).await.unwrap();

        let mut second = request(0);
        second.idempotency_key = Some(key);
        assert!(matches!(
            store.enqueue(second).await,
            Err(CrawlStoreError::IdempotencyConflict(_))
        ));
        reset(&store).await;
    }

    #[tokio::test]
    async fn postgres_claims_each_webhook_lifecycle_event_once() {
        let Some(store) = store() else { return };
        let _guard = DATABASE_LOCK.get_or_init(|| Mutex::new(())).lock().await;
        reset(&store).await;
        let mut crawl_request = request(0);
        crawl_request.webhook = Some(FirecrawlWebhook::Config(
            crate::models::FirecrawlWebhookConfig {
                url: "https://hooks.example.com/crawl".to_string(),
                headers: std::collections::HashMap::new(),
                metadata: std::collections::HashMap::new(),
                events: vec![
                    "started".to_string(),
                    "page".to_string(),
                    "completed".to_string(),
                ],
            },
        ));
        let crawl = store.enqueue(crawl_request).await.unwrap();
        let crawl_id = Uuid::parse_str(&crawl.id).unwrap();
        let task = store.claim_task("worker").await.unwrap().unwrap();
        assert!(store
            .claim_job_webhook(crawl_id, "started")
            .await
            .unwrap()
            .is_some());
        assert!(store
            .claim_job_webhook(crawl_id, "started")
            .await
            .unwrap()
            .is_none());
        store
            .finish_success(&task, page("https://example.com"), vec![])
            .await
            .unwrap();
        assert!(store.claim_page_webhook(&task).await.unwrap().is_some());
        assert!(store.claim_page_webhook(&task).await.unwrap().is_none());
        assert!(store
            .claim_job_webhook(crawl_id, "completed")
            .await
            .unwrap()
            .is_some());
        assert!(store
            .claim_job_webhook(crawl_id, "completed")
            .await
            .unwrap()
            .is_none());
        reset(&store).await;
    }

    #[tokio::test]
    async fn postgres_retries_retryable_failures_before_marking_failed() {
        let Some(store) = store() else { return };
        let _guard = DATABASE_LOCK.get_or_init(|| Mutex::new(())).lock().await;
        reset(&store).await;
        let crawl = store.enqueue(request(1)).await.unwrap();
        let first = store.claim_task("test-worker").await.unwrap().unwrap();
        store
            .finish_failure(
                &first,
                WebExtractError::FetchFailed("temporary".to_string()),
            )
            .await
            .unwrap();

        let row = sqlx::query("SELECT status, attempts, next_attempt_at > now() AS delayed FROM crawl_tasks WHERE crawl_id = $1")
            .bind(Uuid::parse_str(&crawl.id).unwrap())
            .fetch_one(store.pool().unwrap())
            .await
            .unwrap();
        assert_eq!(row.try_get::<String, _>("status").unwrap(), "queued");
        assert_eq!(row.try_get::<i32, _>("attempts").unwrap(), 1);
        assert!(row.try_get::<bool, _>("delayed").unwrap());

        sqlx::query("UPDATE crawl_tasks SET next_attempt_at = now() - interval '1 second' WHERE crawl_id = $1")
            .bind(Uuid::parse_str(&crawl.id).unwrap())
            .execute(store.pool().unwrap())
            .await
            .unwrap();
        let second = store.claim_task("test-worker").await.unwrap().unwrap();
        store
            .finish_failure(
                &second,
                WebExtractError::FetchFailed("temporary".to_string()),
            )
            .await
            .unwrap();
        let status = store
            .get(&crawl.id, CrawlStatusQuery::default())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.status, "completed");
        assert_eq!(status.failed, 1);
        reset(&store).await;
    }

    #[tokio::test]
    async fn postgres_cleanup_removes_expired_crawls_and_tasks() {
        let Some(store) = store() else { return };
        let _guard = DATABASE_LOCK.get_or_init(|| Mutex::new(())).lock().await;
        reset(&store).await;
        let crawl = store.enqueue(request(0)).await.unwrap();
        sqlx::query("UPDATE crawl_jobs SET status = 'completed', expires_at = now() - interval '1 second' WHERE id = $1")
            .bind(Uuid::parse_str(&crawl.id).unwrap())
            .execute(store.pool().unwrap())
            .await
            .unwrap();
        assert_eq!(store.cleanup_expired().await.unwrap(), 1);
        assert!(store
            .get(&crawl.id, CrawlStatusQuery::default())
            .await
            .unwrap()
            .is_none());
        reset(&store).await;
    }

    #[test]
    fn crawl_path_filters_match_paths_or_full_urls() {
        assert!(url_allowed_by_patterns(
            "https://example.com/blog/post",
            &["^/blog".to_string()],
            &["draft$".to_string()],
            false,
        ));
        assert!(!url_allowed_by_patterns(
            "https://example.com/blog/draft",
            &["^/blog".to_string()],
            &["draft$".to_string()],
            false,
        ));
        assert!(!url_allowed_by_patterns(
            "https://other.example/blog/post",
            &[r"^https://example\.com/".to_string()],
            &[],
            true,
        ));
        assert!(validate_path_patterns(&["[".to_string()], "includePaths").is_err());
    }

    #[test]
    fn similar_url_keys_merge_common_page_aliases_but_keep_queries() {
        let aliases = [
            "https://example.com/docs/",
            "http://www.example.com/docs",
            "https://example.com/docs/index.html",
            "http://example.com/docs/index.php",
        ];
        let keys = aliases
            .map(|url| crawl_dedup_key(url, true))
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(keys.len(), 1);
        assert_ne!(
            crawl_dedup_key("https://example.com/docs?a=1", true),
            crawl_dedup_key("https://example.com/docs?a=2", true)
        );
        assert_ne!(
            crawl_dedup_key("https://example.com/docs/", false),
            crawl_dedup_key("http://example.com/docs", false)
        );
    }
}
