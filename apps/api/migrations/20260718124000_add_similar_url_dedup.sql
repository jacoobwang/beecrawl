ALTER TABLE crawl_jobs
  ADD COLUMN IF NOT EXISTS deduplicate_similar_urls BOOLEAN NOT NULL DEFAULT TRUE;

ALTER TABLE crawl_tasks
  ADD COLUMN IF NOT EXISTS dedup_key TEXT;

UPDATE crawl_tasks SET dedup_key = url WHERE dedup_key IS NULL;

ALTER TABLE crawl_tasks ALTER COLUMN dedup_key SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS crawl_tasks_dedup_key_idx
  ON crawl_tasks (crawl_id, dedup_key);
