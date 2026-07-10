CREATE TABLE IF NOT EXISTS crawl_jobs (
  id UUID PRIMARY KEY,
  url TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('queued', 'scraping', 'completed', 'cancelled')),
  page_limit BIGINT NOT NULL CHECK (page_limit > 0),
  max_depth INTEGER NOT NULL CHECK (max_depth >= 0),
  include_subdomains BOOLEAN NOT NULL,
  ignore_query_parameters BOOLEAN NOT NULL,
  timeout_seconds BIGINT NOT NULL,
  wait_for_ms BIGINT NOT NULL,
  use_browser TEXT NOT NULL,
  cancel_requested BOOLEAN NOT NULL DEFAULT FALSE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  started_at TIMESTAMPTZ,
  finished_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS crawl_tasks (
  id UUID PRIMARY KEY,
  crawl_id UUID NOT NULL REFERENCES crawl_jobs(id) ON DELETE CASCADE,
  url TEXT NOT NULL,
  depth INTEGER NOT NULL CHECK (depth >= 0),
  status TEXT NOT NULL CHECK (status IN ('queued', 'active', 'completed', 'failed')),
  attempts INTEGER NOT NULL DEFAULT 0,
  lease_token UUID,
  lease_expires_at TIMESTAMPTZ,
  worker_id TEXT,
  result JSONB,
  error_code TEXT,
  error_message TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  started_at TIMESTAMPTZ,
  finished_at TIMESTAMPTZ,
  UNIQUE (crawl_id, url)
);

CREATE INDEX IF NOT EXISTS crawl_tasks_claim_idx
  ON crawl_tasks (status, created_at)
  WHERE status IN ('queued', 'active');

CREATE INDEX IF NOT EXISTS crawl_tasks_crawl_idx
  ON crawl_tasks (crawl_id, status, finished_at);
