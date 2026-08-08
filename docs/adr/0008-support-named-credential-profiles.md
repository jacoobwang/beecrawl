# Support named Credential Profiles from the first CLI release

BeeCrawl CLI will support multiple named Credential Profiles, with `default` used when no profile is explicitly selected. Profile selection will be available through a command-line flag, `BEECRAWL_PROFILE`, or the locally stored active profile, allowing one installation to switch between workspaces and API endpoints without replacing credentials.
