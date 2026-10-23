from typing import List

from pydantic import BaseModel, parse_raw_as


class Item(BaseModel):
    sku: str
    qty: int


def load_many(raw):
    return parse_raw_as(List[Item], raw)
