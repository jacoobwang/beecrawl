# Keep BeeCrawl CLI independent from the Dashboard

BeeCrawl CLI will live as an independent package in the BeeCrawl repository and will consume the public BeeCrawl API through an API key. It will not depend directly on Dashboard authentication, database tables, frontend code, or internal service routes; this keeps the CLI usable by both humans and coding agents and preserves the Dashboard as a control plane rather than an execution dependency.
