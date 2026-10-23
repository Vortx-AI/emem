from typing import List

from pydantic import BaseModel, model_validator


class Order(BaseModel):
    items: List[float]
    total: float

    @model_validator(mode="after")
    def _check(self):
        if abs(sum(self.items) - self.total) > 1e-9:
            raise ValueError("total does not match items")
        return self
