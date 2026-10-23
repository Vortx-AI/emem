from datetime import datetime
from typing import List

from pydantic import BaseModel


class Event(BaseModel):
    name: str
    at: datetime
    tags: List[str]


def to_canonical_json(ev):
    return ev.json(sort_keys=True, separators=(",", ":"))
