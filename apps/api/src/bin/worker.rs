use beecrawl_api::crawl::{run_worker_forever, CrawlStore};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    CrawlStore::migrate_from_env().await?;
    run_worker_forever(CrawlStore::from_env()).await
}
