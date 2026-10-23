from pydantic import BaseModel


class Item(BaseModel):
    sku: str
    qty: int


def update(item, **changes):
    return Item.parse_obj({**item.dict(), **changes})
