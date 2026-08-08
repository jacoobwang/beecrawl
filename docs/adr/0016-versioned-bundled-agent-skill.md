# Bundle the Agent Skill with the CLI release

The BeeCrawl Agent Skill will ship inside each `beecrawl-cli` package and be installed from that package's versioned asset. `init` will not fetch a latest remote Skill at runtime; it will provide dry-run and explicit overwrite behavior so the installed instructions remain compatible with the CLI version that delivered them.
