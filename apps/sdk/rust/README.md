# BeeCrawl Rust SDK

The Rust SDK is a thin asynchronous HTTP client for BeeCrawl. It does not
embed a browser; rendering, caching, and workers remain on the server.

```toml
[dependencies]
beecrawl-sdk = "0.1"
```

```rust
use beecrawl_sdk::BeeCrawlClient;
use serde_json::json;
use std::time::Duration;

#[tokio::main]
async fn main() -> beecrawl_sdk::Result<()> {
    let client = BeeCrawlClient::builder("http://127.0.0.1:8000")
        .api_key("your-key")
        .build()?;

    let page = client
        .scrape("https://example.com", json!({"formats": ["markdown", "links"]}))
        .await?;
    println!("{}", page["markdown"]);

    let job = client.crawl("https://example.com", json!({"limit": 100})).await?;
    let result = client
        .poll_crawl(&job["id"].as_str().unwrap(), 0, 20,
            Duration::from_secs(2), Duration::from_secs(300))
        .await?;
    println!("{}", result["status"]);
    Ok(())
}
```

The `v2_*`, browser, Agent, and Monitor methods cover the complete public v2
surface. Methods accept `serde_json::Value` options for forward compatibility
and include multipart/reference parsing, job errors and cancellation, browser
replay and handoff, plus monitor updates and checks.
