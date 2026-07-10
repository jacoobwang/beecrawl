use beecrawl_api::{cache::CacheStore, crawl::CrawlStore};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let deleted = CrawlStore::from_env().cleanup_expired().await?;
    let deleted_cache = CacheStore::from_env().cleanup_expired().await;
    println!("deleted {deleted} crawl jobs and {deleted_cache} scrape cache entries");
    Ok(())
}
