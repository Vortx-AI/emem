from pydantic import BaseModel


class Item(BaseModel):
    sku: str
    qty: int


def load(data):
    return Item.parse_obj(data)
