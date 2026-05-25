from __future__ import annotations

import threading
import time
from concurrent import futures

import grpc
import structlog

from src import scraper
from src.db import RecipeDB
from src.proto import recipe_pb2, recipe_pb2_grpc

log = structlog.get_logger()
SCRAPE_DELAY = 2.0


class RecipeServicer(recipe_pb2_grpc.RecipeServiceServicer):
    def __init__(self, db: RecipeDB) -> None:
        self._db = db

    def AddScrapeJob(
        self,
        request: recipe_pb2.AddScrapeJobRequest,
        context: grpc.ServicerContext,
    ) -> recipe_pb2.AddScrapeJobResponse:
        url = request.url.strip()
        if not url:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "url is required")

        self._db.enqueue_url(url)

        if not request.background:
            try:
                recipe = scraper.scrape_recipe(url)
                self._db.save_recipe(recipe)
                log.info("sync scrape done", url=url)
                return recipe_pb2.AddScrapeJobResponse(
                    status="done",
                    recipe=_recipe_to_proto(recipe),
                )
            except scraper.ScrapeError as e:
                log.warning("sync scrape failed", url=url, error=str(e))
                context.abort(grpc.StatusCode.INTERNAL, str(e))

        log.info("job queued", url=url)
        return recipe_pb2.AddScrapeJobResponse(status="queued")

    def SearchRecipes(
        self,
        request: recipe_pb2.SearchRecipesRequest,
        context: grpc.ServicerContext,
    ) -> recipe_pb2.SearchRecipesResponse:
        if not request.query.strip():
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "query is required")

        limit = request.limit if request.limit > 0 else 20
        results = self._db.search(request.query, limit=limit)
        hits = [
            recipe_pb2.RecipeHit(recipe=_recipe_to_proto(r["recipe"]), score=r["score"])
            for r in results
        ]
        log.info("search", query=request.query, hits=len(hits))
        return recipe_pb2.SearchRecipesResponse(hits=hits)

    def GetRecipe(
        self,
        request: recipe_pb2.GetRecipeRequest,
        context: grpc.ServicerContext,
    ) -> recipe_pb2.GetRecipeResponse:
        recipe = self._db.get_recipe(request.url)
        if recipe is None:
            context.abort(grpc.StatusCode.NOT_FOUND, "recipe not found")
        assert recipe is not None
        return _recipe_to_proto(recipe)

    def QueueStatus(
        self,
        request: recipe_pb2.QueueStatusRequest,
        context: grpc.ServicerContext,
    ) -> recipe_pb2.QueueStatusResponse:
        stats = self._db.queue_stats()
        return recipe_pb2.QueueStatusResponse(
            pending=stats["pending"],
            in_progress=stats["in_progress"],
            done=stats["done"],
            error=stats["error"],
        )


def _recipe_to_proto(recipe: dict) -> recipe_pb2.GetRecipeResponse:
    return recipe_pb2.GetRecipeResponse(
        url=recipe.get("url", ""),
        title=recipe.get("title", ""),
        total_time=recipe.get("total_time", 0) or 0,
        ingredients=recipe.get("ingredients", []),
        instructions=recipe.get("instructions", []),
        image=recipe.get("image", ""),
    )


def _scrape_worker(db: RecipeDB, stop_event: threading.Event) -> None:
    worker_log = structlog.get_logger("worker")
    worker_log.info("worker started")

    while not stop_event.is_set():
        job = db.next_pending()
        if job is None:
            wait_start = time.monotonic()
            while not stop_event.is_set() and (time.monotonic() - wait_start) < 5.0:
                time.sleep(0.1)
            continue

        job_id, url = job
        worker_log.info("scraping job", job_id=job_id, url=url)
        try:
            recipe = scraper.scrape_recipe(url)
            db.save_recipe(recipe)
            db.mark_done(job_id)
            worker_log.info(
                "job done", job_id=job_id, url=url, title=recipe.get("title", "")
            )
        except scraper.ScrapeError as e:
            db.mark_error(job_id, str(e))
            worker_log.warning("job failed", job_id=job_id, url=url, error=str(e))

        if not stop_event.is_set():
            time.sleep(SCRAPE_DELAY)

    worker_log.info("worker stopped")


def serve(host: str = "[::]", port: int = 50051, db_path: str = "recipes.db") -> None:
    structlog.configure(
        processors=[
            structlog.stdlib.add_log_level,
            structlog.dev.ConsoleRenderer(),
        ],
    )

    db = RecipeDB(db_path)
    stop_event = threading.Event()

    worker = threading.Thread(target=_scrape_worker, args=(db, stop_event), daemon=True)
    worker.start()

    server = grpc.server(futures.ThreadPoolExecutor(max_workers=10))
    recipe_pb2_grpc.add_RecipeServiceServicer_to_server(RecipeServicer(db), server)
    server.add_insecure_port(f"{host}:{port}")

    log.info("gRPC server starting", host=host, port=port)
    server.start()

    try:
        server.wait_for_termination()
    except KeyboardInterrupt:
        log.info("shutting down")
    finally:
        stop_event.set()
        server.stop(grace=5)
        db.close()
