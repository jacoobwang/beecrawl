ALTER TABLE crawl_jobs
  ADD COLUMN IF NOT EXISTS allow_external_links BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS crawl_entire_domain BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS sitemap TEXT NOT NULL DEFAULT 'include'
    CHECK (sitemap IN ('skip', 'include', 'only'));
