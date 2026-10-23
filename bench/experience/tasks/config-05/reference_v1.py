from pydantic import BaseModel


class Counter(BaseModel):
    n: int = 0

    class Config:
        allow_mutation = False


def bump(c):
    return Counter(n=c.n + 1)
