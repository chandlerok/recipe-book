"""Recipe scraper gRPC service with DuckDB + full-text search."""

import argparse

from src.server import serve


def main() -> None:
    parser = argparse.ArgumentParser(description="Recipe scraper gRPC service")
    parser.add_argument("--host", default="[::]", help="Bind address")
    parser.add_argument("--port", type=int, default=50051, help="Bind port")
    parser.add_argument("--db", default="recipes.db", help="DuckDB database path")
    args = parser.parse_args()

    serve(host=args.host, port=args.port, db_path=args.db)


if __name__ == "__main__":
    main()
