use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use thiserror::Error;
use tokio::time::sleep;
use uuid::Uuid;

use crate::cache::CacheStore;
use crate::models::{FirecrawlWebhook, SearchRequest, WebExtractScrapeRequest};
use crate::{search, web_extract};

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error(
        "workflow_storage_unavailable: Set DATABASE_URL to enable Agent and Monitor workflows"
    )]
    StorageUnavailable,
    #[error("workflow_database_error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("workflow_not_found")]
    NotFound,
}

#[derive(Clone, Default)]
pub struct WorkflowStore {
    pool: Option<PgPool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCreateRequest {
    pub prompt: String,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default = "default_agent_budget")]
    pub max_credits: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJob {
    pub success: bool,
    pub id: Uuid,
    pub status: String,
    pub prompt: String,
    pub budget: usize,
    pub credits_used: usize,
    pub sources: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorCreateRequest {
    pub name: String,
    pub url: String,
    #[serde(default = "default_schedule_seconds")]
    pub schedule_seconds: u64,
    pub webhook: Option<FirecrawlWebhook>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorUpdateRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub schedule_seconds: Option<u64>,
    pub webhook: Option<FirecrawlWebhook>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub schedule_seconds: u64,
    pub enabled: bool,
    pub next_run_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorCheck {
    pub id: Uuid,
    pub monitor_id: Uuid,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_diff: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_diff: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

fn default_agent_budget() -> usize {
    5
}
fn default_schedule_seconds() -> u64 {
    3600
}
fn default_true() -> bool {
    true
}

impl WorkflowStore {
    pub fn from_env() -> Self {
        let pool = std::env::var("BEECRAWL_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .ok()
            .map(|url| {
                PgPool::connect_lazy(&url).expect("DATABASE_URL must be a valid PostgreSQL URL")
            });
        Self { pool }
    }

    pub fn with_pool(pool: PgPool) -> Self {
        Self { pool: Some(pool) }
    }

    fn pool(&self) -> Result<&PgPool, WorkflowError> {
        self.pool.as_ref().ok_or(WorkflowError::StorageUnavailable)
    }

    pub async fn create_agent(
        &self,
        owner: &str,
        request: AgentCreateRequest,
    ) -> Result<Uuid, WorkflowError> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO agent_jobs (id, owner_key, prompt, urls, budget) VALUES ($1, $2, $3, $4, $5)")
            .bind(id).bind(owner).bind(request.prompt).bind(json!(request.urls))
            .bind(request.max_credits as i32).execute(self.pool()?).await?;
        Ok(id)
    }

    pub async fn agent(&self, owner: &str, id: Uuid) -> Result<AgentJob, WorkflowError> {
        let row = sqlx::query("SELECT * FROM agent_jobs WHERE id = $1 AND owner_key = $2")
            .bind(id)
            .bind(owner)
            .fetch_optional(self.pool()?)
            .await?
            .ok_or(WorkflowError::NotFound)?;
        Ok(agent_from_row(&row))
    }

    pub async fn cancel_agent(&self, owner: &str, id: Uuid) -> Result<AgentJob, WorkflowError> {
        sqlx::query("UPDATE agent_jobs SET cancel_requested = true, status = CASE WHEN status = 'queued' THEN 'cancelled' ELSE status END, finished_at = CASE WHEN status = 'queued' THEN now() ELSE finished_at END WHERE id = $1 AND owner_key = $2")
            .bind(id).bind(owner).execute(self.pool()?).await?;
        self.agent(owner, id).await
    }

    pub async fn create_monitor(
        &self,
        owner: &str,
        request: MonitorCreateRequest,
    ) -> Result<Monitor, WorkflowError> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO monitors (id, owner_key, name, url, schedule_seconds, enabled) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(id).bind(owner).bind(&request.name).bind(&request.url)
            .bind(request.schedule_seconds as i32).bind(request.enabled)
            .execute(self.pool()?).await?;
        if let Some(webhook) = request.webhook {
            sqlx::query("UPDATE monitors SET webhook = $2 WHERE id = $1")
                .bind(id)
                .bind(serde_json::to_value(webhook).expect("webhook serializes"))
                .execute(self.pool()?)
                .await?;
        }
        self.monitor(owner, id).await
    }

    pub async fn monitors(&self, owner: &str) -> Result<Vec<Monitor>, WorkflowError> {
        let rows =
            sqlx::query("SELECT * FROM monitors WHERE owner_key = $1 ORDER BY created_at DESC")
                .bind(owner)
                .fetch_all(self.pool()?)
                .await?;
        Ok(rows.iter().map(monitor_from_row).collect())
    }

    pub async fn monitor(&self, owner: &str, id: Uuid) -> Result<Monitor, WorkflowError> {
        let row = sqlx::query("SELECT * FROM monitors WHERE id = $1 AND owner_key = $2")
            .bind(id)
            .bind(owner)
            .fetch_optional(self.pool()?)
            .await?
            .ok_or(WorkflowError::NotFound)?;
        Ok(monitor_from_row(&row))
    }

    pub async fn update_monitor(
        &self,
        owner: &str,
        id: Uuid,
        request: MonitorUpdateRequest,
    ) -> Result<Monitor, WorkflowError> {
        let webhook = request
            .webhook
            .map(|value| serde_json::to_value(value).expect("webhook serializes"));
        let row = sqlx::query("UPDATE monitors SET name = COALESCE($3, name), url = COALESCE($4, url), schedule_seconds = COALESCE($5, schedule_seconds), webhook = COALESCE($6, webhook), enabled = COALESCE($7, enabled), updated_at = now() WHERE id = $1 AND owner_key = $2 RETURNING id")
            .bind(id).bind(owner).bind(request.name).bind(request.url)
            .bind(request.schedule_seconds.map(|value| value as i32)).bind(webhook).bind(request.enabled)
            .fetch_optional(self.pool()?).await?;
        if row.is_none() {
            return Err(WorkflowError::NotFound);
        }
        self.monitor(owner, id).await
    }

    pub async fn delete_monitor(&self, owner: &str, id: Uuid) -> Result<(), WorkflowError> {
        let result = sqlx::query("DELETE FROM monitors WHERE id = $1 AND owner_key = $2")
            .bind(id)
            .bind(owner)
            .execute(self.pool()?)
            .await?;
        if result.rows_affected() == 0 {
            return Err(WorkflowError::NotFound);
        }
        Ok(())
    }

    pub async fn run_monitor(&self, owner: &str, id: Uuid) -> Result<Uuid, WorkflowError> {
        self.monitor(owner, id).await?;
        self.enqueue_check(id).await
    }

    async fn enqueue_check(&self, monitor_id: Uuid) -> Result<Uuid, WorkflowError> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO monitor_checks (id, monitor_id) VALUES ($1, $2)")
            .bind(id)
            .bind(monitor_id)
            .execute(self.pool()?)
            .await?;
        Ok(id)
    }

    pub async fn checks(
        &self,
        owner: &str,
        monitor_id: Uuid,
    ) -> Result<Vec<MonitorCheck>, WorkflowError> {
        self.monitor(owner, monitor_id).await?;
        let rows = sqlx::query("SELECT checks.* FROM monitor_checks checks WHERE monitor_id = $1 ORDER BY created_at DESC LIMIT 100")
            .bind(monitor_id).fetch_all(self.pool()?).await?;
        Ok(rows.iter().map(check_from_row).collect())
    }

    pub async fn check(
        &self,
        owner: &str,
        monitor_id: Uuid,
        check_id: Uuid,
    ) -> Result<MonitorCheck, WorkflowError> {
        self.monitor(owner, monitor_id).await?;
        let row = sqlx::query("SELECT * FROM monitor_checks WHERE id = $1 AND monitor_id = $2")
            .bind(check_id)
            .bind(monitor_id)
            .fetch_optional(self.pool()?)
            .await?
            .ok_or(WorkflowError::NotFound)?;
        Ok(check_from_row(&row))
    }

    async fn schedule_due(&self) -> Result<bool, WorkflowError> {
        let row = sqlx::query("WITH due AS (SELECT id, schedule_seconds FROM monitors WHERE enabled = true AND next_run_at <= now() ORDER BY next_run_at FOR UPDATE SKIP LOCKED LIMIT 1), advanced AS (UPDATE monitors SET next_run_at = now() + make_interval(secs => due.schedule_seconds), updated_at = now() FROM due WHERE monitors.id = due.id RETURNING monitors.id) INSERT INTO monitor_checks (id, monitor_id) SELECT $1, id FROM advanced RETURNING monitor_id")
            .bind(Uuid::new_v4()).fetch_optional(self.pool()?).await?;
        Ok(row.is_some())
    }

    async fn claim_agent(&self) -> Result<Option<ClaimedAgent>, WorkflowError> {
        let row = sqlx::query("WITH candidate AS (SELECT id FROM agent_jobs WHERE status = 'queued' AND cancel_requested = false ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE agent_jobs SET status = 'running', started_at = now() FROM candidate WHERE agent_jobs.id = candidate.id RETURNING agent_jobs.id, prompt, urls, budget")
            .fetch_optional(self.pool()?).await?;
        row.map(|row| {
            Ok(ClaimedAgent {
                id: row.try_get("id")?,
                prompt: row.try_get("prompt")?,
                urls: serde_json::from_value(row.try_get("urls")?).unwrap_or_default(),
                budget: row.try_get::<i32, _>("budget")? as usize,
            })
        })
        .transpose()
    }

    async fn claim_check(&self) -> Result<Option<ClaimedCheck>, WorkflowError> {
        let row = sqlx::query("WITH candidate AS (SELECT checks.id FROM monitor_checks checks WHERE checks.status = 'queued' ORDER BY checks.created_at FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE monitor_checks SET status = 'running', started_at = now() FROM candidate, monitors WHERE monitor_checks.id = candidate.id AND monitors.id = monitor_checks.monitor_id RETURNING monitor_checks.id, monitor_checks.monitor_id, monitors.url, monitors.webhook")
            .fetch_optional(self.pool()?).await?;
        row.map(|row| {
            Ok(ClaimedCheck {
                id: row.try_get("id")?,
                monitor_id: row.try_get("monitor_id")?,
                url: row.try_get("url")?,
                webhook: row
                    .try_get::<Option<Value>, _>("webhook")?
                    .and_then(|value| serde_json::from_value(value).ok()),
            })
        })
        .transpose()
    }

    pub async fn process_once(
        &self,
        client: &reqwest::Client,
        cache: &CacheStore,
    ) -> Result<bool, WorkflowError> {
        if self.schedule_due().await? {
            return Ok(true);
        }
        if let Some(agent) = self.claim_agent().await? {
            self.process_agent(client, cache, agent).await?;
            return Ok(true);
        }
        if let Some(check) = self.claim_check().await? {
            self.process_check(client, cache, check).await?;
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn queue_depth(&self) -> Result<u64, WorkflowError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM agent_jobs WHERE status IN ('queued', 'running')) + (SELECT COUNT(*) FROM monitor_checks WHERE status IN ('queued', 'running'))",
        )
        .fetch_one(self.pool()?)
        .await?;
        Ok(count.max(0) as u64)
    }

    async fn process_agent(
        &self,
        client: &reqwest::Client,
        cache: &CacheStore,
        agent: ClaimedAgent,
    ) -> Result<(), WorkflowError> {
        let urls = if agent.urls.is_empty() {
            search::search(
                client,
                SearchRequest {
                    query: agent.prompt.clone(),
                    limit: agent.budget,
                    lang: "en".to_string(),
                    country: "us".to_string(),
                    scrape_options: None,
                    categories: vec![],
                    include_domains: vec![],
                    exclude_domains: vec![],
                    tbs: None,
                    location: None,
                    filter: None,
                    async_scraping: false,
                    highlights: false,
                },
            )
            .await
            .map(|response| {
                response
                    .results
                    .into_iter()
                    .map(|result| result.url)
                    .collect()
            })
            .unwrap_or_default()
        } else {
            agent.urls
        };
        let mut sources = Vec::new();
        for url in urls.into_iter().take(agent.budget) {
            let cancelled: bool =
                sqlx::query_scalar("SELECT cancel_requested FROM agent_jobs WHERE id = $1")
                    .bind(agent.id)
                    .fetch_one(self.pool()?)
                    .await?;
            if cancelled {
                sqlx::query("UPDATE agent_jobs SET status = 'cancelled', finished_at = now(), sources = $2, used = $3 WHERE id = $1")
                    .bind(agent.id).bind(json!(sources)).bind(sources.len() as i32).execute(self.pool()?).await?;
                return Ok(());
            }
            match scrape_markdown(client, cache, &url).await {
                Ok(page) => sources.push(json!({"url": page.final_url, "title": page.metadata.title, "markdown": page.markdown})),
                Err(error) => sources.push(json!({"url": url, "error": error.to_string()})),
            }
        }
        let excerpts = sources
            .iter()
            .filter_map(|source| source.get("markdown").and_then(Value::as_str))
            .map(|text| text.chars().take(1200).collect::<String>())
            .collect::<Vec<_>>();
        let result = json!({"prompt": agent.prompt, "answer": excerpts.join("\n\n"), "sourceCount": sources.len()});
        sqlx::query("UPDATE agent_jobs SET status = 'completed', finished_at = now(), sources = $2, result = $3, used = $4 WHERE id = $1")
            .bind(agent.id).bind(json!(sources)).bind(result).bind(sources.len() as i32).execute(self.pool()?).await?;
        Ok(())
    }

    async fn process_check(
        &self,
        client: &reqwest::Client,
        cache: &CacheStore,
        check: ClaimedCheck,
    ) -> Result<(), WorkflowError> {
        match scrape_markdown(client, cache, &check.url).await {
            Ok(page) => {
                let previous = sqlx::query("SELECT text_content, json_content FROM monitor_checks WHERE monitor_id = $1 AND status = 'completed' ORDER BY finished_at DESC LIMIT 1")
                    .bind(check.monitor_id).fetch_optional(self.pool()?).await?;
                let old_text = previous
                    .as_ref()
                    .and_then(|row| row.try_get::<Option<String>, _>("text_content").ok())
                    .flatten()
                    .unwrap_or_default();
                let old_json = previous
                    .as_ref()
                    .and_then(|row| row.try_get::<Option<Value>, _>("json_content").ok())
                    .flatten()
                    .unwrap_or_else(|| json!({}));
                let current_json = json!({"url": page.final_url, "title": page.metadata.title, "markdown": page.markdown});
                let text_diff = git_style_diff(
                    &old_text,
                    current_json["markdown"].as_str().unwrap_or_default(),
                );
                let json_diff = json_changes(&old_json, &current_json);
                sqlx::query("UPDATE monitor_checks SET status = 'completed', snapshot = $2, text_content = $3, json_content = $4, text_diff = $5, json_diff = $6, finished_at = now() WHERE id = $1")
                    .bind(check.id).bind(&current_json).bind(current_json["markdown"].as_str()).bind(&current_json).bind(&text_diff).bind(&json_diff).execute(self.pool()?).await?;
                if let Some(webhook) = check.webhook {
                    if crate::webhook::deliver(client, &webhook, "monitor", check.monitor_id, "completed", true, json!({"checkId": check.id, "snapshot": current_json, "textDiff": text_diff, "jsonDiff": json_diff}), None).await.is_ok() {
                        sqlx::query("UPDATE monitor_checks SET webhook_delivered_at = now() WHERE id = $1").bind(check.id).execute(self.pool()?).await?;
                    }
                }
            }
            Err(error) => {
                let message = error.to_string();
                sqlx::query("UPDATE monitor_checks SET status = 'failed', error = $2, finished_at = now() WHERE id = $1")
                    .bind(check.id).bind(&message).execute(self.pool()?).await?;
                if let Some(webhook) = check.webhook {
                    let _ = crate::webhook::deliver(
                        client,
                        &webhook,
                        "monitor",
                        check.monitor_id,
                        "failed",
                        false,
                        json!({"checkId": check.id}),
                        Some(&message),
                    )
                    .await;
                }
            }
        }
        Ok(())
    }
}

pub async fn run_worker_forever(store: WorkflowStore) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let cache = CacheStore::from_env();
    loop {
        if !store.process_once(&client, &cache).await? {
            sleep(Duration::from_millis(500)).await;
        }
    }
}

struct ClaimedAgent {
    id: Uuid,
    prompt: String,
    urls: Vec<String>,
    budget: usize,
}
struct ClaimedCheck {
    id: Uuid,
    monitor_id: Uuid,
    url: String,
    webhook: Option<FirecrawlWebhook>,
}

async fn scrape_markdown(
    client: &reqwest::Client,
    cache: &CacheStore,
    url: &str,
) -> Result<crate::models::WebExtractScrapeResponse, web_extract::WebExtractError> {
    web_extract::scrape_with_cache(
        client,
        cache,
        WebExtractScrapeRequest {
            url: url.to_string(),
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
    .await
}

fn agent_from_row(row: &sqlx::postgres::PgRow) -> AgentJob {
    AgentJob {
        success: true,
        id: row.get("id"),
        status: row.get("status"),
        prompt: row.get("prompt"),
        budget: row.get::<i32, _>("budget") as usize,
        credits_used: row.get::<i32, _>("used") as usize,
        sources: row.get("sources"),
        data: row.get("result"),
        error: row.get("error"),
        created_at: row.get("created_at"),
        finished_at: row.get("finished_at"),
    }
}
fn monitor_from_row(row: &sqlx::postgres::PgRow) -> Monitor {
    Monitor {
        id: row.get("id"),
        name: row.get("name"),
        url: row.get("url"),
        schedule_seconds: row.get::<i32, _>("schedule_seconds") as u64,
        enabled: row.get("enabled"),
        next_run_at: row.get("next_run_at"),
        created_at: row.get("created_at"),
    }
}
fn check_from_row(row: &sqlx::postgres::PgRow) -> MonitorCheck {
    MonitorCheck {
        id: row.get("id"),
        monitor_id: row.get("monitor_id"),
        status: row.get("status"),
        snapshot: row.get("snapshot"),
        text_diff: row.get("text_diff"),
        json_diff: row.get("json_diff"),
        error: row.get("error"),
        created_at: row.get("created_at"),
        finished_at: row.get("finished_at"),
    }
}

pub fn git_style_diff(before: &str, after: &str) -> String {
    if before == after {
        return String::new();
    }
    let before_lines = before.lines().collect::<BTreeSet<_>>();
    let after_lines = after.lines().collect::<BTreeSet<_>>();
    let mut result = vec!["--- previous".to_string(), "+++ current".to_string()];
    result.extend(
        before_lines
            .difference(&after_lines)
            .map(|line| format!("-{line}")),
    );
    result.extend(
        after_lines
            .difference(&before_lines)
            .map(|line| format!("+{line}")),
    );
    result.join("\n")
}

pub fn json_changes(before: &Value, after: &Value) -> Value {
    fn walk(path: &str, before: &Value, after: &Value, changes: &mut Vec<Value>) {
        if before == after {
            return;
        }
        match (before, after) {
            (Value::Object(left), Value::Object(right)) => {
                let keys = left.keys().chain(right.keys()).collect::<BTreeSet<_>>();
                for key in keys { walk(&format!("{path}/{key}"), left.get(key).unwrap_or(&Value::Null), right.get(key).unwrap_or(&Value::Null), changes); }
            }
            _ => changes.push(json!({"path": if path.is_empty() { "/" } else { path }, "before": before, "after": after})),
        }
    }
    let mut changes = Vec::new();
    walk("", before, after, &mut changes);
    Value::Array(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_tracking_reports_text_and_json_changes() {
        let text = git_style_diff("alpha\nbeta", "alpha\ngamma");
        assert!(text.contains("-beta"));
        assert!(text.contains("+gamma"));
        let changes = json_changes(&json!({"title": "old"}), &json!({"title": "new"}));
        assert_eq!(changes[0]["path"], "/title");
    }

    #[tokio::test]
    async fn postgres_persists_owned_agent_and_monitor_jobs() {
        let Ok(url) = std::env::var("BEECRAWL_TEST_DATABASE_URL") else {
            return;
        };
        let pool = PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let store = WorkflowStore::with_pool(pool);
        let agent_id = store
            .create_agent(
                "owner-a",
                AgentCreateRequest {
                    prompt: "Summarize".to_string(),
                    urls: vec!["https://example.com".to_string()],
                    max_credits: 1,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            store.agent("owner-a", agent_id).await.unwrap().status,
            "queued"
        );
        assert!(matches!(
            store.agent("owner-b", agent_id).await,
            Err(WorkflowError::NotFound)
        ));
        assert_eq!(
            store
                .cancel_agent("owner-a", agent_id)
                .await
                .unwrap()
                .status,
            "cancelled"
        );

        let monitor = store
            .create_monitor(
                "owner-a",
                MonitorCreateRequest {
                    name: "Example".to_string(),
                    url: "https://example.com".to_string(),
                    schedule_seconds: 3600,
                    webhook: None,
                    enabled: true,
                },
            )
            .await
            .unwrap();
        let check_id = store.run_monitor("owner-a", monitor.id).await.unwrap();
        assert!(store
            .checks("owner-a", monitor.id)
            .await
            .unwrap()
            .iter()
            .any(|check| check.id == check_id));
        store.delete_monitor("owner-a", monitor.id).await.unwrap();
    }
}
