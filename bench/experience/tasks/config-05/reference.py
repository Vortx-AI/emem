from pydantic import BaseModel, ConfigDict


class Counter(BaseModel):
    model_config = ConfigDict(frozen=True)

    n: int = 0


def bump(c):
    return Counter(n=c.n + 1)
