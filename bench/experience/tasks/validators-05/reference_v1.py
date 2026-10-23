from typing import List

from pydantic import BaseModel, root_validator


class Order(BaseModel):
    items: List[float]
    total: float

    @root_validator
    def _check(cls, values):
        items, total = values.get("items"), values.get("total")
        if items is not None and total is not None and abs(sum(items) - total) > 1e-9:
            raise ValueError("total does not match items")
        return values
