from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class AddScrapeJobRequest(_message.Message):
    __slots__ = ("url", "background")
    URL_FIELD_NUMBER: _ClassVar[int]
    BACKGROUND_FIELD_NUMBER: _ClassVar[int]
    url: str
    background: bool
    def __init__(self, url: _Optional[str] = ..., background: bool = ...) -> None: ...

class AddScrapeJobResponse(_message.Message):
    __slots__ = ("status", "recipe")
    STATUS_FIELD_NUMBER: _ClassVar[int]
    RECIPE_FIELD_NUMBER: _ClassVar[int]
    status: str
    recipe: GetRecipeResponse
    def __init__(self, status: _Optional[str] = ..., recipe: _Optional[_Union[GetRecipeResponse, _Mapping]] = ...) -> None: ...

class SearchRecipesRequest(_message.Message):
    __slots__ = ("query", "limit")
    QUERY_FIELD_NUMBER: _ClassVar[int]
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    query: str
    limit: int
    def __init__(self, query: _Optional[str] = ..., limit: _Optional[int] = ...) -> None: ...

class SearchRecipesResponse(_message.Message):
    __slots__ = ("hits",)
    HITS_FIELD_NUMBER: _ClassVar[int]
    hits: _containers.RepeatedCompositeFieldContainer[RecipeHit]
    def __init__(self, hits: _Optional[_Iterable[_Union[RecipeHit, _Mapping]]] = ...) -> None: ...

class RecipeHit(_message.Message):
    __slots__ = ("recipe", "score")
    RECIPE_FIELD_NUMBER: _ClassVar[int]
    SCORE_FIELD_NUMBER: _ClassVar[int]
    recipe: GetRecipeResponse
    score: float
    def __init__(self, recipe: _Optional[_Union[GetRecipeResponse, _Mapping]] = ..., score: _Optional[float] = ...) -> None: ...

class GetRecipeRequest(_message.Message):
    __slots__ = ("url",)
    URL_FIELD_NUMBER: _ClassVar[int]
    url: str
    def __init__(self, url: _Optional[str] = ...) -> None: ...

class GetRecipeResponse(_message.Message):
    __slots__ = ("url", "title", "total_time", "ingredients", "instructions", "image")
    URL_FIELD_NUMBER: _ClassVar[int]
    TITLE_FIELD_NUMBER: _ClassVar[int]
    TOTAL_TIME_FIELD_NUMBER: _ClassVar[int]
    INGREDIENTS_FIELD_NUMBER: _ClassVar[int]
    INSTRUCTIONS_FIELD_NUMBER: _ClassVar[int]
    IMAGE_FIELD_NUMBER: _ClassVar[int]
    url: str
    title: str
    total_time: int
    ingredients: _containers.RepeatedScalarFieldContainer[str]
    instructions: _containers.RepeatedScalarFieldContainer[str]
    image: str
    def __init__(self, url: _Optional[str] = ..., title: _Optional[str] = ..., total_time: _Optional[int] = ..., ingredients: _Optional[_Iterable[str]] = ..., instructions: _Optional[_Iterable[str]] = ..., image: _Optional[str] = ...) -> None: ...

class QueueStatusRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class QueueStatusResponse(_message.Message):
    __slots__ = ("pending", "in_progress", "done", "error")
    PENDING_FIELD_NUMBER: _ClassVar[int]
    IN_PROGRESS_FIELD_NUMBER: _ClassVar[int]
    DONE_FIELD_NUMBER: _ClassVar[int]
    ERROR_FIELD_NUMBER: _ClassVar[int]
    pending: int
    in_progress: int
    done: int
    error: int
    def __init__(self, pending: _Optional[int] = ..., in_progress: _Optional[int] = ..., done: _Optional[int] = ..., error: _Optional[int] = ...) -> None: ...
