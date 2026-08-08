# Adopt the accepted CLI design baseline and staged delivery

BeeCrawl CLI will be delivered as a TypeScript npm package over the v2 Node SDK, with Dashboard-mediated PKCE login, named local profiles, task-oriented commands, strict stdout/stderr separation, and a bundled versioned Agent Skill. The first implementation stage covers search, scrape, map, extract, crawl, and agent; parse, browser/interact, monitor, and batch scrape remain reserved extensions so the command model can grow without destabilizing the initial agent workflow.
