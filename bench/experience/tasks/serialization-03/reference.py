from typing import Optional

from pydantic import BaseModel


class Patch(BaseModel):
    name: Optional[str] = None
    age: Optional[int] = None
    nickname: Optional[str] = None


def patch_body(p):
    return p.dict(exclude_unset=True)
