use beecrawl_api::crawl::CrawlStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    CrawlStore::migrate_from_env().await
}
