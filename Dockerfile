# Stage 1: Build server
FROM rust:latest AS builder
RUN apt-get update && apt-get install -y \
  cmake \
  pkg-config \
  clang \
  golang-go \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY recipe-scraper-server/ .
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/recipe-scraper-server /usr/local/bin/
COPY recipe-scraper-server/static /app/static
WORKDIR /app
EXPOSE 3000
ENV HOST=0.0.0.0
CMD ["recipe-scraper-server"]
