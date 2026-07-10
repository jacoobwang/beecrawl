#!/usr/bin/env bash
set -euo pipefail

if ! command -v sqlx >/dev/null 2>&1; then
  echo "sqlx-cli is required: cargo install sqlx-cli --no-default-features --features postgres,rustls" >&2
  exit 1
fi

if [[ -z "${DATABASE_URL:-}" ]]; then
  export DATABASE_URL="${BEECRAWL_DATABASE_URL:-}"
fi
if [[ -z "$DATABASE_URL" ]]; then
  echo "Set BEECRAWL_DATABASE_URL or DATABASE_URL" >&2
  exit 1
fi

exec sqlx "$@"
