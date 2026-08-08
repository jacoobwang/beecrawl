# Build the CLI as a TypeScript npm package over the Node SDK

BeeCrawl CLI will be implemented in TypeScript for Node.js, published as `beecrawl-cli` with a `beecrawl` binary, and layered on top of the existing `beecrawl-sdk` package. Reusing the SDK keeps HTTP behavior, authentication headers, errors, and job polling consistent across programmatic and terminal clients.
