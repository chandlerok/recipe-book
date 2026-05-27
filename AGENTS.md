# AGENTS.md — Recipe Book

## Project Structure

```
recipe-scraper-server/    → Axum monolith (crawler + scraper + API + cron)
recipe-scraper-web/       → SolidJS frontend (search box + results list)
```

## Commands

### Development
- `mise run dev` — Run both server and frontend in dev mode
- `mise run build` — Build both server and frontend for production
- `mise run test` — Run Rust tests

### Individual Commands
- `cargo run` — Run the Axum server (from recipe-scraper-server/)
- `cargo test` — Run Rust tests (from recipe-scraper-server/)
- `bun run dev` — Run SolidJS frontend (from recipe-scraper-web/)
- `bun run build` — Build SolidJS frontend (from recipe-scraper-web/)

## Architecture Notes

- The server uses `wreq` (Chrome110 emulation) for scraping individual recipe pages
- The crawler uses `reqwest` for sitemap crawling
- Crawling runs on a daily cron schedule (midnight)
- The crawler only enqueues URLs — background workers handle scraping
- Full-text search uses PostgreSQL tsvector with weighted ranking (title=A, ingredients=B, instructions=C)
- Swagger UI is available at `/docs/`
- The SolidJS frontend is served as static files with SPA fallback
