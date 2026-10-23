from typing import List

from pydantic import BaseModel
from solution import nested_model_names


class Inner(BaseModel):
    a: int


class Leaf(BaseModel):
    z: int


class Outer(BaseModel):
    inner: Inner
    items: List[Leaf]


class Flat(BaseModel):
    x: int


def test_nested():
    assert nested_model_names(Outer) == ["Inner", "Leaf"]


def test_flat():
    assert nested_model_names(Flat) == []
