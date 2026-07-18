CREATE TABLE IF NOT EXISTS agent_jobs (
  id UUID PRIMARY KEY,
  owner_key TEXT NOT NULL,
  prompt TEXT NOT NULL,
  urls JSONB NOT NULL DEFAULT '[]'::jsonb,
  status TEXT NOT NULL DEFAULT 'queued',
  budget INTEGER NOT NULL DEFAULT 5 CHECK (budget > 0 AND budget <= 100),
  used INTEGER NOT NULL DEFAULT 0,
  sources JSONB NOT NULL DEFAULT '[]'::jsonb,
  result JSONB,
  error TEXT,
  cancel_requested BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  started_at TIMESTAMPTZ,
  finished_at TIMESTAMPTZ,
  expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '7 days'
);

CREATE INDEX IF NOT EXISTS agent_jobs_claim_idx
  ON agent_jobs (status, created_at) WHERE status = 'queued';

CREATE TABLE IF NOT EXISTS monitors (
  id UUID PRIMARY KEY,
  owner_key TEXT NOT NULL,
  name TEXT NOT NULL,
  url TEXT NOT NULL,
  schedule_seconds INTEGER NOT NULL CHECK (schedule_seconds >= 60),
  enabled BOOLEAN NOT NULL DEFAULT true,
  webhook JSONB,
  next_run_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS monitors_due_idx
  ON monitors (next_run_at) WHERE enabled = true;

CREATE TABLE IF NOT EXISTS monitor_checks (
  id UUID PRIMARY KEY,
  monitor_id UUID NOT NULL REFERENCES monitors(id) ON DELETE CASCADE,
  status TEXT NOT NULL DEFAULT 'queued',
  snapshot JSONB,
  text_content TEXT,
  json_content JSONB,
  text_diff TEXT,
  json_diff JSONB,
  error TEXT,
  webhook_delivered_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  started_at TIMESTAMPTZ,
  finished_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS monitor_checks_claim_idx
  ON monitor_checks (status, created_at) WHERE status = 'queued';
CREATE INDEX IF NOT EXISTS monitor_checks_history_idx
  ON monitor_checks (monitor_id, created_at DESC);
