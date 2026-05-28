# Stage 1: Build frontend
FROM oven/bun:1 AS frontend
WORKDIR /build
COPY recipe-scraper-web/package.json recipe-scraper-web/bun.lock ./
RUN bun install
COPY recipe-scraper-web/ .
RUN bun run build

# Stage 2: Build server
FROM rust:1.85 AS builder
RUN apt-get update && apt-get install -y \
  cmake \
  pkg-config \
  clang \
  golang-go \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY recipe-scraper-server/ .
RUN cargo build --release

# Stage 3: Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/recipe-scraper-server /usr/local/bin/
COPY --from=frontend /build/dist /usr/share/recipe-book/web
EXPOSE 3000
ENV HOST=0.0.0.0
ENV WEB_DIST=/usr/share/recipe-book/web
CMD ["recipe-scraper-server"]
