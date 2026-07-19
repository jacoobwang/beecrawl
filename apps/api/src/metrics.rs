use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::body::Body;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

static API_REQUESTS: AtomicU64 = AtomicU64::new(0);
static API_FAILURES: AtomicU64 = AtomicU64::new(0);
static API_LATENCY_MICROS: AtomicU64 = AtomicU64::new(0);
static ENGINE_HTTP: AtomicU64 = AtomicU64::new(0);
static ENGINE_BROWSER: AtomicU64 = AtomicU64::new(0);
static ENGINE_TLS: AtomicU64 = AtomicU64::new(0);
static ENGINE_FALLBACKS: AtomicU64 = AtomicU64::new(0);

pub async fn middleware(request: Request<Body>, next: Next) -> Response {
    let started = Instant::now();
    let response = next.run(request).await;
    API_REQUESTS.fetch_add(1, Ordering::Relaxed);
    API_LATENCY_MICROS.fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
    if response.status().is_client_error() || response.status().is_server_error() {
        API_FAILURES.fetch_add(1, Ordering::Relaxed);
    }
    response
}

pub fn record_engine(provider: &str, fallback: bool) {
    match provider {
        "bee_engine" => &ENGINE_BROWSER,
        "tls_client" => &ENGINE_TLS,
        _ => &ENGINE_HTTP,
    }
    .fetch_add(1, Ordering::Relaxed);
    if fallback {
        ENGINE_FALLBACKS.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn prometheus(crawl_queue: u64, workflow_queue: u64) -> String {
    let requests = API_REQUESTS.load(Ordering::Relaxed);
    let failures = API_FAILURES.load(Ordering::Relaxed);
    let latency_seconds = API_LATENCY_MICROS.load(Ordering::Relaxed) as f64 / 1_000_000.0;
    format!(
        "# TYPE beecrawl_api_requests_total counter\nbeecrawl_api_requests_total {requests}\n\
# TYPE beecrawl_api_failures_total counter\nbeecrawl_api_failures_total {failures}\n\
# TYPE beecrawl_api_latency_seconds summary\nbeecrawl_api_latency_seconds_count {requests}\nbeecrawl_api_latency_seconds_sum {latency_seconds}\n\
# TYPE beecrawl_engine_selections_total counter\nbeecrawl_engine_selections_total{{engine=\"http\"}} {}\nbeecrawl_engine_selections_total{{engine=\"browser\"}} {}\nbeecrawl_engine_selections_total{{engine=\"tls_client\"}} {}\n\
# TYPE beecrawl_engine_fallbacks_total counter\nbeecrawl_engine_fallbacks_total {}\n\
# TYPE beecrawl_queue_depth gauge\nbeecrawl_queue_depth{{queue=\"crawl\"}} {crawl_queue}\nbeecrawl_queue_depth{{queue=\"workflow\"}} {workflow_queue}\n",
        ENGINE_HTTP.load(Ordering::Relaxed),
        ENGINE_BROWSER.load(Ordering::Relaxed),
        ENGINE_TLS.load(Ordering::Relaxed),
        ENGINE_FALLBACKS.load(Ordering::Relaxed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prometheus_output_contains_all_required_metric_families() {
        record_engine("bee_engine", true);
        let output = prometheus(3, 2);
        for name in [
            "api_latency_seconds",
            "engine_selections_total",
            "queue_depth",
            "api_failures_total",
        ] {
            assert!(output.contains(name));
        }
        assert!(output.contains("queue=\"crawl\"} 3"));
    }
}
