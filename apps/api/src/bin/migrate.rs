use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let database_url =
        std::env::var("BEECRAWL_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL"))?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    pool.close().await;
    Ok(())
}
