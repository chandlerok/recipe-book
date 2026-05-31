# AGENTS.md — Recipe Book

## Project Structure

```
recipe-scraper-server/    → Axum monolith (crawler + scraper + API + cron)
recipe-scraper-web/       → SolidJS frontend (search box + results list)
```

## Commands

Tasks are defined as executable scripts in `.mise/tasks/`. Run them with `mise run <task>`.

### Development
- `mise run dev` — Run both server and frontend in dev mode
- `mise run build` — Build both server and frontend for production
- `mise run test` — Run Rust tests
- `mise run pre-commit` — Run all checks (run automatically by git hooks)

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
- Search uses an in-memory tantivy index (`src/search.rs`) built from all recipes on startup
- Swagger UI is available at `/docs/`
- The SolidJS frontend is served as static files with SPA fallback

### Search Heuristics

Search is implemented in `src/search.rs` using tantivy's BM25 scoring.

**Index schema** (stored fields for response, text fields for search):
- `url` — stored string, used for document identity
- `title` — text field (boost 3.0)
- `ingredients` — text field (boost 1.5)
- `instructions` — text field (boost 1.0)
- `publication` — stored string, used for publication boost queries

**Query structure** (nested boolean):
1. **Must group** — at least one text/fuzzy clause must match (provides relevance floor):
   - **Short queries** (1-2 words): OR across all fields permissive — the QueryParser with default occurrence catches any matching term
   - **Long queries** (3+ words): AND filter — every word must appear in at least one of the edge-ngram indexed fields (`title_ngram`, `ingredients_ngram`). The `prefix_only(2, 20)` tokenizer generates prefix n-grams, so "sou" (n-grams: `so`, `sou`) matches "soup" (n-grams: `so`, `sou`, `sou`, `soup`), and "chick" matches "chicken" — without needing separate fuzzy or prefix queries.
2. **Should** — score-only boosts that don't affect filtering:
   - Exact phrase query in title with slop 1 (boost 5.0) — heavily favors consecutive word matches, so "French Onion Soup" beats "French Onion Cabbage Soup"
   - Publication boost for known sites (Bon Appétit, NYT Cooking, Epicurious, boost 0.3)

**Result fetching**: After the tantivy search returns matching URLs, full recipe data (ingredients/instructions as arrays) is fetched from PostgreSQL via `WHERE url IN (...)` to populate the response.

## Deployment

The app is deployed via Dokploy as a Docker Compose stack behind Tailscale.

**Stack**: `docker-compose.yaml` at repo root — defines `postgres` (alpine) and `app` (builds from `Dockerfile`) services. Port 3001 maps to the app's internal port 3000.

**Auto-deploy**: Every push to `main` triggers a rebuild and redeploy. The Dokploy instance checks the git remote for changes on a schedule.

**Caching**: Subsequent builds are fast — Docker caches layers (Rust compilation, frontend build). Only changed layers rebuild.

### Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `port is already allocated` | Host port 3000 is taken by Dokploy itself | The stack uses port 3001 instead; check docker-compose.yaml |
| `SSH connection error` | Dokploy can't reach the target server | Verify the server IP in Dokploy dashboard matches the host machine's reachable address (e.g. Tailscale IP) |
| `Authentication failed: Invalid SSH private key` | SSH key mismatch between Dokploy and server's authorized_keys | Regenerate SSH key in Dokploy, add the new public key to `~/.ssh/authorized_keys` on the server |
| `Compose file not found` | Wrong compose file path or extension | Check `composePath` in Dokploy — must match the filename (`.yaml` vs `.yml`) |
| `lockfile had changes` | `bun.lock` out of sync with `package.json` between environments | Remove `--frozen-lockfile` from Dockerfile or regenerate lockfile locally |
| `rustc X is not supported` | Docker base image has older Rust than the lockfile requires | Update `FROM rust:` tag in Dockerfile to `rust:latest` |
| Build fails in Dokploy container, works locally | Shell mismatch (e.g., `fish` vs `bash`) | Ensure the deploy user's login shell on the server is `bash` |
| `Remote command failed with exit code 1` | Permission issue on the server | Check Docker group membership (`groups`) and `/etc/dokploy/` directory permissions |
