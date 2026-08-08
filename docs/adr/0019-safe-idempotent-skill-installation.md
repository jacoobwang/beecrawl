# Make Skill installation safe and idempotent

`init` will update files previously marked as BeeCrawl-managed, skip same-name unmanaged files by default, and expose `--dry-run` plus explicit `--force` behavior. This makes repeated setup and CLI upgrades predictable without silently overwriting user-authored Agent instructions.
