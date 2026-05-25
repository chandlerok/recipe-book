from __future__ import annotations

from typing import cast
from unittest.mock import MagicMock

import grpc
import pytest

from src.proto import recipe_pb2
from src.server import RecipeServicer, _recipe_to_proto


class MockContext:
    def __init__(self) -> None:
        self._aborted: tuple[grpc.StatusCode, str] | None = None

    def abort(self, code: grpc.StatusCode, detail: str) -> None:
        self._aborted = (code, detail)
        raise grpc.RpcError(f"aborted: {detail}")

    def was_aborted_with(self, code: grpc.StatusCode) -> bool:
        return self._aborted is not None and self._aborted[0] == code


@pytest.fixture
def db() -> MagicMock:
    return MagicMock()


@pytest.fixture
def ctx() -> MockContext:
    return MockContext()


@pytest.fixture
def servicer(db: MagicMock) -> RecipeServicer:
    return RecipeServicer(db)


class TestAddScrapeJob:
    def test_adds_url_synchronously(
        self, servicer: RecipeServicer, db: MagicMock, ctx: MockContext
    ) -> None:
        db.enqueue_url.return_value = "pending"
        db.get_recipe.return_value = None

        request = recipe_pb2.AddScrapeJobRequest(
            url="https://example.com/recipe", background=False
        )

        with pytest.raises(grpc.RpcError):
            servicer.AddScrapeJob(request, cast(grpc.ServicerContext, ctx))

    def test_queues_url_in_background_mode(
        self, servicer: RecipeServicer, db: MagicMock, ctx: MockContext
    ) -> None:
        db.enqueue_url.return_value = "pending"

        request = recipe_pb2.AddScrapeJobRequest(
            url="https://example.com/recipe", background=True
        )

        response = servicer.AddScrapeJob(request, cast(grpc.ServicerContext, ctx))
        assert response.status == "queued"
        db.enqueue_url.assert_called_once_with("https://example.com/recipe")

    def test_empty_url_aborts(
        self, servicer: RecipeServicer, db: MagicMock, ctx: MockContext
    ) -> None:
        request = recipe_pb2.AddScrapeJobRequest(url="   ", background=True)

        with pytest.raises(grpc.RpcError):
            servicer.AddScrapeJob(request, cast(grpc.ServicerContext, ctx))
        assert ctx.was_aborted_with(grpc.StatusCode.INVALID_ARGUMENT)


class TestSearchRecipes:
    def test_returns_hits(
        self, servicer: RecipeServicer, db: MagicMock, ctx: MockContext
    ) -> None:
        db.search.return_value = [
            {
                "recipe": {
                    "url": "https://example.com/1",
                    "title": "Chicken Tacos",
                    "total_time": 30,
                    "ingredients": ["chicken"],
                    "instructions": ["cook"],
                    "image": "",
                },
                "score": 0.5,
            }
        ]

        request = recipe_pb2.SearchRecipesRequest(query="chicken", limit=10)
        response = servicer.SearchRecipes(request, cast(grpc.ServicerContext, ctx))

        assert len(response.hits) == 1
        assert response.hits[0].recipe.title == "Chicken Tacos"
        assert response.hits[0].score == 0.5

    def test_empty_query_aborts(
        self, servicer: RecipeServicer, db: MagicMock, ctx: MockContext
    ) -> None:
        request = recipe_pb2.SearchRecipesRequest(query="   ", limit=10)

        with pytest.raises(grpc.RpcError):
            servicer.SearchRecipes(request, cast(grpc.ServicerContext, ctx))
        assert ctx.was_aborted_with(grpc.StatusCode.INVALID_ARGUMENT)

    def test_defaults_limit_to_20_when_zero(
        self, servicer: RecipeServicer, db: MagicMock, ctx: MockContext
    ) -> None:
        db.search.return_value = []

        request = recipe_pb2.SearchRecipesRequest(query="chicken", limit=0)
        servicer.SearchRecipes(request, cast(grpc.ServicerContext, ctx))

        _, kwargs = db.search.call_args
        assert kwargs["limit"] == 20


class TestGetRecipe:
    def test_returns_recipe_when_found(
        self, servicer: RecipeServicer, db: MagicMock, ctx: MockContext
    ) -> None:
        db.get_recipe.return_value = {
            "url": "https://example.com/1",
            "title": "Chicken Tacos",
            "total_time": 30,
            "ingredients": ["chicken"],
            "instructions": ["cook"],
            "image": "https://img.example.com/1.jpg",
        }

        request = recipe_pb2.GetRecipeRequest(url="https://example.com/1")
        response = servicer.GetRecipe(request, cast(grpc.ServicerContext, ctx))

        assert response.title == "Chicken Tacos"
        assert response.total_time == 30

    def test_not_found_aborts(
        self, servicer: RecipeServicer, db: MagicMock, ctx: MockContext
    ) -> None:
        db.get_recipe.return_value = None

        request = recipe_pb2.GetRecipeRequest(url="https://example.com/missing")

        with pytest.raises(grpc.RpcError):
            servicer.GetRecipe(request, cast(grpc.ServicerContext, ctx))
        assert ctx.was_aborted_with(grpc.StatusCode.NOT_FOUND)


class TestQueueStatus:
    def test_returns_stats(
        self, servicer: RecipeServicer, db: MagicMock, ctx: MockContext
    ) -> None:
        db.queue_stats.return_value = {
            "pending": 3,
            "in_progress": 1,
            "done": 10,
            "error": 2,
        }

        request = recipe_pb2.QueueStatusRequest()
        response = servicer.QueueStatus(request, cast(grpc.ServicerContext, ctx))

        assert response.pending == 3
        assert response.in_progress == 1
        assert response.done == 10
        assert response.error == 2


class TestRecipeToProto:
    def test_converts_dict_to_proto(self) -> None:
        recipe = {
            "url": "https://example.com/test",
            "title": "Test Dish",
            "total_time": 45,
            "ingredients": ["a", "b"],
            "instructions": ["step 1", "step 2"],
            "image": "https://img.example.com/test.jpg",
        }

        proto = _recipe_to_proto(recipe)

        assert proto.url == "https://example.com/test"
        assert proto.title == "Test Dish"
        assert proto.total_time == 45
        assert list(proto.ingredients) == ["a", "b"]
        assert list(proto.instructions) == ["step 1", "step 2"]
        assert proto.image == "https://img.example.com/test.jpg"

    def test_defaults_missing_fields(self) -> None:
        proto = _recipe_to_proto({"url": "https://example.com/minimal"})
        assert proto.url == "https://example.com/minimal"
        assert proto.title == ""
        assert proto.total_time == 0
        assert list(proto.ingredients) == []
        assert list(proto.instructions) == []
        assert proto.image == ""
