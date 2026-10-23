from pydantic import BaseModel


class Point(BaseModel):
    x: int
    y: int

    class Config:
        frozen = True
