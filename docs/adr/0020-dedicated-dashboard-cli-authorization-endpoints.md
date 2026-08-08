# Add a dedicated Dashboard authorization flow for the CLI

BeeCrawl CLI login will use dedicated Dashboard authorization and token-exchange endpoints rather than scraping or automating the existing key-management UI. The Dashboard will remain responsible for Clerk authentication, workspace authorization, and CLI key creation, while the CLI receives only the result of a short-lived authorization transaction.
