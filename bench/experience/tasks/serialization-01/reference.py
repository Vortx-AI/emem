from typing import Optional

from pydantic import BaseModel


class User(BaseModel):
    id: int
    name: str
    email: Optional[str] = None
    password: str


def to_payload(user):
    return user.dict(exclude_none=True, exclude={"password"})
