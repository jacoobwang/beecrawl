use beecrawl_api::crawl::{run_worker_forever, CrawlStore};
use beecrawl_api::workflows::{run_worker_forever as run_workflows, WorkflowStore};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tokio::try_join!(
        run_worker_forever(CrawlStore::from_env()),
        run_workflows(WorkflowStore::from_env())
    )?;
    Ok(())
}
