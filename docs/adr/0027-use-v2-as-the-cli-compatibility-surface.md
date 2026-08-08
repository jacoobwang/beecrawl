# Use the v2 API surface as the CLI compatibility boundary

BeeCrawl CLI will use the Node SDK's v2 methods for its user-facing commands and hide API versioning from the command names. Legacy SDK methods and endpoint shapes remain available to existing programmatic clients but are not used as an implicit CLI fallback, so command output and Job semantics stay consistent.
