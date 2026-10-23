from typing import List, Optional

from pydantic import BaseModel


class Filters(BaseModel):
    tags: Optional[List[str]]
    limit: Optional[int]
    q: Optional[str]


def active(f):
    return {k: v for k, v in f.dict().items() if v is not None}
