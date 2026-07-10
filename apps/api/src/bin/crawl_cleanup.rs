use beecrawl_api::crawl::CrawlStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let deleted = CrawlStore::from_env().cleanup_expired().await?;
    println!("deleted {deleted} expired crawl jobs");
    Ok(())
}
