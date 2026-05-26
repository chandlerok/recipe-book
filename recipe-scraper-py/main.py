"""Recipe scraper gRPC service with PostgreSQL + full-text search."""

import argparse

from src.server import serve


def main() -> None:
    parser = argparse.ArgumentParser(description="Recipe scraper gRPC service")
    parser.add_argument("--host", default="[::]", help="Bind address")
    parser.add_argument("--port", type=int, default=50051, help="Bind port")
    parser.add_argument(
        "--pg-dsn",
        default="postgresql:///recipe_book",
        help="PostgreSQL connection string",
    )
    args = parser.parse_args()

    serve(host=args.host, port=args.port, dsn=args.pg_dsn)


if __name__ == "__main__":
    main()
