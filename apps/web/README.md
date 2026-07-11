# BeeCrawl Website

Astro static site for the BeeCrawl website.

## Local development

From the repository root:

```sh
pnpm install
pnpm web:dev
```

## Cloudflare Pages

Create a Pages project connected to this repository with these settings:

- Framework preset: `Astro`
- Root directory: `apps/web`
- Build command: `pnpm build`
- Build output directory: `dist`

The site uses Astro static output, so no Cloudflare adapter is required.
