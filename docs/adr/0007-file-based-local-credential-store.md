# Persist CLI credentials in a protected local configuration file

The CLI will persist profiles and API keys in the platform's standard per-user configuration directory, with restrictive permissions rather than requiring an OS keychain. This keeps login usable in headless agents, containers, and CI while still preventing normal other-user reads; environment variables and explicit flags remain available when no local file is appropriate.
