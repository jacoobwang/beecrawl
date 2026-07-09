from pydantic import BaseModel, Field, HttpUrl


class ScrapeRequest(BaseModel):
    url: HttpUrl


class Link(BaseModel):
    text: str
    url: str


class ScrapeResponse(BaseModel):
    url: str
    title: str | None = None
    text: str
    links: list[Link] = Field(default_factory=list)
    metadata: dict[str, str] = Field(default_factory=dict)


class ExtractRequest(BaseModel):
    url: HttpUrl
    schema_: dict[str, str] = Field(alias="schema")


class ExtractResponse(BaseModel):
    url: str
    data: dict[str, str | None]
    scrape: ScrapeResponse
