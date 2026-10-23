from typing import Optional

from pydantic import BaseModel


class Address(BaseModel):
    city: str
    zip: Optional[str]


class User(BaseModel):
    name: str
    address: Optional[Address]


def city_of(data):
    u = User.parse_obj(data)
    return u.address.city if u.address else None
