from typing import Optional

from pydantic import BaseModel


class Rating(BaseModel):
    item: str
    score: Optional[int] = ...
