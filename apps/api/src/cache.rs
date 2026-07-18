use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

use crate::models::ProviderPage;

#[derive(Clone)]
pub struct CacheStore {
    pool: Result<PgPool, String>,
}

impl CacheStore {
    pub fn from_env() -> Self {
        let url = std::env::var("BEECRAWL_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL"));
        match url {
            Ok(url) => Self::from_database_url(&url),
            Err(error) => Self {
                pool: Err(error.to_string()),
            },
        }
    }

    pub fn from_database_url(url: &str) -> Self {
        Self {
            pool: PgPoolOptions::new()
                .max_connections(5)
                .connect_lazy(url)
                .map_err(|error| error.to_string()),
        }
    }

    pub async fn get(
        &self,
        key: &str,
        max_age_seconds: u64,
        require_screenshot: bool,
    ) -> Option<ProviderPage> {
        let pool = self.pool.as_ref().ok()?;
        let row = sqlx::query(
            "SELECT url, final_url, html, status_code, title, language, provider, rendered, screenshot FROM scrape_cache WHERE cache_key = $1 AND fetched_at >= now() - make_interval(secs => $2) AND expires_at > now() AND ($3 = false OR screenshot IS NOT NULL)",
        )
        .bind(key)
        .bind(max_age_seconds as i64)
        .bind(require_screenshot)
        .fetch_optional(pool)
        .await
        .ok()??;
        Some(ProviderPage {
            url: row.try_get("url").ok()?,
            final_url: row.try_get("final_url").ok()?,
            html: row.try_get("html").ok()?,
            status_code: row
                .try_get::<Option<i32>, _>("status_code")
                .ok()?
                .map(|value| value as u16),
            title: row.try_get("title").ok()?,
            language: row.try_get("language").ok()?,
            provider: row.try_get("provider").ok()?,
            rendered: row.try_get("rendered").ok()?,
            screenshot: row.try_get("screenshot").ok()?,
            engine_outcomes: vec![],
            fallback_reason: Some("served from scrape cache".to_string()),
            proxy_used: false,
        })
    }

    pub async fn put(&self, key: &str, page: &ProviderPage) {
        let Some(pool) = self.pool.as_ref().ok() else {
            return;
        };
        let retention_days = std::env::var("BEECRAWL_SCRAPE_CACHE_RETENTION_DAYS")
            .ok()
            .and_then(|value| value.parse::<i32>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(7);
        let _ = sqlx::query(
            "INSERT INTO scrape_cache (cache_key, url, final_url, html, status_code, title, language, provider, rendered, screenshot, fetched_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now(), now() + make_interval(days => $11)) ON CONFLICT (cache_key) DO UPDATE SET url = EXCLUDED.url, final_url = EXCLUDED.final_url, html = EXCLUDED.html, status_code = EXCLUDED.status_code, title = EXCLUDED.title, language = EXCLUDED.language, provider = EXCLUDED.provider, rendered = EXCLUDED.rendered, screenshot = EXCLUDED.screenshot, fetched_at = now(), expires_at = EXCLUDED.expires_at",
        )
        .bind(key)
        .bind(&page.url)
        .bind(&page.final_url)
        .bind(&page.html)
        .bind(page.status_code.map(i32::from))
        .bind(&page.title)
        .bind(&page.language)
        .bind(&page.provider)
        .bind(page.rendered)
        .bind(&page.screenshot)
        .bind(retention_days)
        .execute(pool)
        .await;
    }

    pub async fn cleanup_expired(&self) -> u64 {
        let Some(pool) = self.pool.as_ref().ok() else {
            return 0;
        };
        sqlx::query("DELETE FROM scrape_cache WHERE expires_at <= now()")
            .execute(pool)
            .await
            .map(|result| result.rows_affected())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn postgres_cache_round_trip() {
        let Ok(url) = std::env::var("BEECRAWL_TEST_DATABASE_URL") else {
            return;
        };
        let store = CacheStore::from_database_url(&url);
        let key = "test:scrape-cache-round-trip";
        sqlx::query("DELETE FROM scrape_cache WHERE cache_key = $1")
            .bind(key)
            .execute(store.pool.as_ref().unwrap())
            .await
            .unwrap();
        let page = ProviderPage {
            url: "https://example.com".to_string(),
            final_url: "https://example.com/".to_string(),
            html: "<main>cached</main>".to_string(),
            status_code: Some(200),
            title: Some("Example".to_string()),
            language: Some("en".to_string()),
            provider: "http_static".to_string(),
            rendered: false,
            screenshot: Some("cached-image".to_string()),
            engine_outcomes: vec![],
            fallback_reason: None,
            proxy_used: false,
        };
        store.put(key, &page).await;
        let cached = store.get(key, 3600, true).await.unwrap();
        assert_eq!(cached.html, page.html);
        assert_eq!(cached.screenshot, page.screenshot);
        sqlx::query("DELETE FROM scrape_cache WHERE cache_key = $1")
            .bind(key)
            .execute(store.pool.as_ref().unwrap())
            .await
            .unwrap();
    }
}
