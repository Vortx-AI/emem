from typing import List

from pydantic import BaseModel, validator


class Article(BaseModel):
    title: str
    tags: List[str]

    @validator("tags", pre=True)
    def _split(cls, v):
        if isinstance(v, str):
            return [p.strip() for p in v.split(",") if p.strip()]
        return v

    @validator("tags", each_item=True)
    def _lower(cls, v):
        return v.lower()
