CREATE TABLE IF NOT EXISTS scrape_cache (
  cache_key TEXT PRIMARY KEY,
  url TEXT NOT NULL,
  final_url TEXT NOT NULL,
  html TEXT NOT NULL,
  status_code INTEGER,
  title TEXT,
  language TEXT,
  provider TEXT NOT NULL,
  rendered BOOLEAN NOT NULL,
  screenshot TEXT,
  fetched_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS scrape_cache_expiry_idx
  ON scrape_cache (expires_at);
