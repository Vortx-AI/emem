from typing import Optional

from pydantic import BaseModel


class Person(BaseModel):
    first: str
    last: str
    middle: Optional[str]
