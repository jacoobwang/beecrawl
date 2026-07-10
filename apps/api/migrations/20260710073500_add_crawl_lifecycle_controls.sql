ALTER TABLE crawl_jobs
  ADD COLUMN max_retries INTEGER NOT NULL DEFAULT 2 CHECK (max_retries >= 0),
  ADD COLUMN expires_at TIMESTAMPTZ;

ALTER TABLE crawl_tasks
  ADD COLUMN next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now();

CREATE INDEX crawl_jobs_expiry_idx
  ON crawl_jobs (expires_at)
  WHERE expires_at IS NOT NULL;

CREATE INDEX crawl_tasks_retry_idx
  ON crawl_tasks (next_attempt_at, created_at)
  WHERE status = 'queued';
