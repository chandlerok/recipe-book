# Recipe Book

A recipe search engine that crawls popular food blogs and provides full-text search with fuzzy matching and typeahead results.

## Architecture

```
recipe-scraper-server/    → Axum monolith (crawler + scraper + API + cron)
recipe-scraper-web/       → SolidJS frontend (search box + results list)
```

### Server

Built with [Axum](https://github.com/tokio-rs/axum) (Rust), the server combines three components:

- **Crawler** — Discovers recipe URLs from 14 food blog sitemaps daily at midnight using `reqwest` with rate limiting and proxy support
- **Scraper** — Scrapes recipe pages using `wreq` with Chrome 110 TLS emulation, extracts Schema.org JSON-LD recipe data
- **API** — REST endpoints for search, recipe retrieval, and queue management with auto-generated OpenAPI docs via `utoipa`

### Search

PostgreSQL full-text search with three matching strategies combined:

| Strategy | Example | Method |
|---|---|---|
| Exact/stemmed | `chicken` → "Chicken" | `tsvector` + `websearch_to_tsquery` |
| Partial word | `chick` → "Chicken" | `ILIKE` with `pg_trgm` GIN index |
| Fuzzy/spelling | `chikcen` → "Chicken" | `%` similarity operator with trigram index |

Results are scored by title (weight A), ingredients (B), and instructions (C), with similarity bonuses for fuzzy matches. An in-memory cache (30s TTL) absorbs repeated queries.

### Frontend

A [SolidJS](https://solidjs.com/) single-page app with a search box and debounced results list. Built with Vite and TypeScript, served by the Axum server as static files.

## Quick Start

### Prerequisites

- [Rust](https://rustup.rs/)
- [Bun](https://bun.sh/)
- [PostgreSQL](https://postgresql.org/) (running on default port)
- [mise](https://mise.jdx.dev/) (for task runner)

### Setup

```bash
# Install tools
mise install

# Create the database
createdb recipe_book

# Start the server
mise run dev-server

# In another terminal, start the frontend
mise run dev-web
```

Open `http://localhost:5173` for the frontend or `http://localhost:3000/docs` for Swagger UI.

### Commands

Run with `mise run <task>`:

| Task | Description |
|---|---|
| `dev` | Run server and frontend concurrently |
| `build` | Build both for production |
| `test` | Run all tests (63 passing) |
| `pre-commit` | Format, lint, typecheck, and test |

### API Endpoints

| Method | Path | Description |
|---|---|---|
| GET | `/api/recipes/search?q=chicken&limit=20` | Full-text recipe search |
| GET | `/api/recipes?url=https://...` | Get a single recipe by URL |
| POST | `/api/scrape` | Enqueue a URL for background scraping |
| GET | `/api/queue/status` | Queue statistics |
| GET | `/docs/` | Swagger UI |

## Crawled Sites

AllRecipes, BonAppétit, Food52, Simply Recipes, The Pioneer Woman, Taste of Home, Serious Eats, Cookie and Kate, Pinch of Yum, Half Baked Harvest, Love and Lemons, RecipeTin Eats, Minimalist Baker, Budget Bytes

## Tech Stack

- **Backend**: Rust, Axum, sqlx, wreq/reqwest, tokio, utoipa
- **Frontend**: SolidJS, TypeScript, Vite
- **Database**: PostgreSQL (tsvector + pg_trgm)
- **Infrastructure**: mise (task runner), cron (daily crawl)
