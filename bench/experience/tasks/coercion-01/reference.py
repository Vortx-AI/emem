from pydantic import BaseModel


class Row(BaseModel):
    id: int
    price: float
    active: bool


def parse_row(d):
    return Row(**d)
