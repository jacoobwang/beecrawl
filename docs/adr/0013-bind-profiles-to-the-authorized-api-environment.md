# Bind each profile to the API environment returned by Dashboard authorization

The Dashboard authorization exchange will return the canonical BeeCrawl API URL together with the CLI API key. The CLI will persist both values in the profile, default to the production Dashboard only when no environment is specified, and allow explicit Dashboard URL overrides for self-hosted and local installations.
