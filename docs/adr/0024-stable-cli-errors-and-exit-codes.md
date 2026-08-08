# Use stable CLI exit codes and structured diagnostics

BeeCrawl CLI will map usage, authentication, not-found, rate-limit, transport/server, and Job failure/timeout conditions to stable process exit codes instead of exposing raw HTTP statuses. Successful data remains on stdout; diagnostics go to stderr, with `--json` producing a stable structured error object that preserves server details and request identifiers.
