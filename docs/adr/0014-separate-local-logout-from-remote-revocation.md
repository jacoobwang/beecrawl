# Keep CLI logout local-only

`beecrawl logout` will remove the selected profile from the local credential store without requiring network access. The CLI will not revoke remote CLI API keys; remote key lifecycle remains a Dashboard concern, so logout stays offline-safe and cannot accidentally mutate server-side credentials.
