# BeeCrawl Helm Chart

This chart deploys the three BeeCrawl runtime components:

- `beecrawl-api`: Rust HTTP API
- `beecrawl-worker`: Rust crawl and batch worker
- `beecrawl-bee-engine`: Python Playwright browser service

PostgreSQL is not part of this chart. Configure `BEECRAWL_DATABASE_URL` in an
existing Secret and point it at the dedicated `beecrawl` database on the
Aliyun PostgreSQL instance. Do not point it at the `workus` database.

Create the database Secret before installing:

```bash
kubectl create secret generic beecrawl-database-secret \
  --from-literal=BEECRAWL_DATABASE_URL='postgres://user:password@aliyun-rds-host:5432/beecrawl'
```

Optional API and LLM secrets can contain `BEECRAWL_WEB_EXTRACT_API_KEY`,
`BEECRAWL_LLM_API_KEY`, `BEECRAWL_LLM_PROVIDER`, `BEECRAWL_LLM_BASE_URL`, and
`BEECRAWL_LLM_MODEL`.

Install with image tags supplied by CI:

```bash
helm upgrade --install beecrawl infra/charts/beecrawl \
  --namespace beecrawl --create-namespace \
  -f infra/charts/beecrawl/values-production.yaml \
  --set api.ingress.host=beecrawl.example.com \
  --set global.imageTag=sha-abcdef1
```

The migration hook is disabled by default. Run `make migrate-up` from a
network location that can reach Aliyun PostgreSQL, or enable the hook only
after publishing a migration image and configuring `migration.command` and
`migration.args`.
