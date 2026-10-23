from typing import Optional

from pydantic import BaseModel


class Profile(BaseModel):
    name: str
    nickname: Optional[str]
