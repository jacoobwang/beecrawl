use beecrawl_api::crawl::{run_worker_forever, CrawlStore};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_worker_forever(CrawlStore::from_env()).await
}
