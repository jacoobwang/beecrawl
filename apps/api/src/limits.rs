use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header::RETRY_AFTER, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<String, KeyState>>>,
    requests_per_minute: usize,
    max_concurrency: usize,
}

#[derive(Default)]
struct KeyState {
    requests: VecDeque<Instant>,
    active: usize,
    last_seen: Option<Instant>,
}

pub struct RatePermit {
    limiter: RateLimiter,
    key: String,
}

#[derive(Debug, PartialEq)]
pub enum LimitError {
    RateLimited(u64),
    ConcurrencyLimited,
}

impl RateLimiter {
    pub fn from_env() -> Self {
        Self::new(
            env_usize("BEECRAWL_RATE_LIMIT_PER_MINUTE", 120),
            env_usize("BEECRAWL_MAX_CONCURRENCY_PER_KEY", 16),
        )
    }

    pub fn new(requests_per_minute: usize, max_concurrency: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            requests_per_minute: requests_per_minute.max(1),
            max_concurrency: max_concurrency.max(1),
        }
    }

    pub fn try_acquire(&self, key: String) -> Result<RatePermit, LimitError> {
        let now = Instant::now();
        let mut states = self.inner.lock().expect("rate limiter lock poisoned");
        states.retain(|_, state| {
            state.active > 0
                || state
                    .last_seen
                    .is_some_and(|last| now.duration_since(last) < Duration::from_secs(300))
        });
        let state = states.entry(key.clone()).or_default();
        while state
            .requests
            .front()
            .is_some_and(|started| now.duration_since(*started) >= Duration::from_secs(60))
        {
            state.requests.pop_front();
        }
        if state.active >= self.max_concurrency {
            return Err(LimitError::ConcurrencyLimited);
        }
        if state.requests.len() >= self.requests_per_minute {
            let retry = 60_u64.saturating_sub(
                state
                    .requests
                    .front()
                    .map(|started| now.duration_since(*started).as_secs())
                    .unwrap_or(0),
            );
            return Err(LimitError::RateLimited(retry.max(1)));
        }
        state.requests.push_back(now);
        state.active += 1;
        state.last_seen = Some(now);
        Ok(RatePermit {
            limiter: self.clone(),
            key,
        })
    }
}

impl Drop for RatePermit {
    fn drop(&mut self) {
        if let Ok(mut states) = self.limiter.inner.lock() {
            if let Some(state) = states.get_mut(&self.key) {
                state.active = state.active.saturating_sub(1);
                state.last_seen = Some(Instant::now());
            }
        }
    }
}

pub async fn middleware(
    State(limiter): State<RateLimiter>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if matches!(request.uri().path(), "/health" | "/metrics") {
        return next.run(request).await;
    }
    let credential = request
        .headers()
        .get("authorization")
        .or_else(|| request.headers().get("x-api-key"))
        .or_else(|| request.headers().get("x-web-extract-api-key"))
        .and_then(|value| value.to_str().ok())
        .unwrap_or("anonymous");
    let key = format!("{:x}", Sha256::digest(credential.as_bytes()));
    match limiter.try_acquire(key) {
        Ok(_permit) => next.run(request).await,
        Err(error) => {
            let (message, retry_after) = match error {
                LimitError::RateLimited(seconds) => ("Per-key request rate exceeded", seconds),
                LimitError::ConcurrencyLimited => ("Per-key concurrency exceeded", 1),
            };
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"success": false, "error": message})),
            )
                .into_response();
            response.headers_mut().insert(
                RETRY_AFTER,
                HeaderValue::from_str(&retry_after.to_string())
                    .unwrap_or_else(|_| HeaderValue::from_static("1")),
            );
            response
        }
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_enforces_per_key_rate_and_concurrency_independently() {
        let limiter = RateLimiter::new(2, 1);
        let first = limiter.try_acquire("a".to_string()).unwrap();
        assert_eq!(
            limiter.try_acquire("a".to_string()).err(),
            Some(LimitError::ConcurrencyLimited)
        );
        assert!(limiter.try_acquire("b".to_string()).is_ok());
        drop(first);
        drop(limiter.try_acquire("a".to_string()).unwrap());
        assert!(matches!(
            limiter.try_acquire("a".to_string()),
            Err(LimitError::RateLimited(_))
        ));
    }
}
