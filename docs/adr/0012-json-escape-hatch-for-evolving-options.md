# Keep an explicit JSON escape hatch for evolving API options

BeeCrawl CLI will expose common request fields as discoverable flags while accepting complete JSON through `--options`, `--options-file`, or stdin. The escape hatch preserves access to new or less common API fields without requiring a CLI release for every server-side option, while the flag layer keeps common agent and human workflows readable.
