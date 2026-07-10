# Deployment preparation

BeeCrawl uses a public source repository and deploys directly to ACK with
Helm. Runtime credentials stay in GitHub Environments and Kubernetes Secrets.

## 1. Database

Create a dedicated `beecrawl` database on the existing PostgreSQL instance.
The deployment tooling rejects URLs that point to another database.

## 2. ACK Secrets

Configure kubectl for the target ACK cluster, export the required values, and
run the preparation script:

```bash
export NAMESPACE=beecrawl
export BEECRAWL_DATABASE_URL='postgres://user:password@host:5432/beecrawl'
export BEECRAWL_WEB_EXTRACT_API_KEY='replace-me'

# Optional LLM configuration
export BEECRAWL_LLM_API_KEY='replace-me'
export BEECRAWL_LLM_PROVIDER='openai-compatible'
export BEECRAWL_LLM_BASE_URL='https://api.openai.com/v1'
export BEECRAWL_LLM_MODEL='gpt-4o-mini'

./scripts/deploy/prepare-ack.sh
```

For private GHCR images, also export `GHCR_USERNAME` and a read-only
`GHCR_TOKEN`. The script then creates `regcred-ghcr`; pass that name as the
`image_pull_secret` workflow input. Public images do not require it.

The script updates only Secrets whose corresponding environment variables are
present. It never deletes existing optional Secrets.

## 3. GitHub Environment

Create `development` and/or `production` GitHub Environments. Store the ACK
kubeconfig as a `KUBECONFIG` Environment Secret, either raw YAML or base64.
Use environment protection rules for production approval.

## 4. Deploy

Run `Build and publish images`, then dispatch `Deploy to ACK` with the emitted
`sha-*` image tag. The deployment runs the SQLx migration Job before rolling
out the API, worker, and Bee Engine.
