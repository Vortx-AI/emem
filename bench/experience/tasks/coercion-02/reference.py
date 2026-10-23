from pydantic import BaseModel, StrictInt, StrictStr


class Cart(BaseModel):
    count: StrictInt
    label: StrictStr
