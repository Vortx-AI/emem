from pydantic import BaseModel, Field, constr
from solution import max_length_of


class A(BaseModel):
    a: constr(max_length=5)
    b: str = Field("x", max_length=10)
    c: str


def test_lengths():
    assert max_length_of(A, "a") == 5
    assert max_length_of(A, "b") == 10


def test_none():
    assert max_length_of(A, "c") is None
