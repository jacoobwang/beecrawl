ALTER TABLE crawl_jobs
  ADD COLUMN job_type TEXT NOT NULL DEFAULT 'crawl'
  CHECK (job_type IN ('crawl', 'batch_scrape'));
