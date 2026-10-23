from typing import List, Optional

from pydantic import BaseModel
from solution import required_fields


class A(BaseModel):
    id: int
    title: str
    done: bool = False


class B(BaseModel):
    x: int = 1
    y: List[str] = []


class C(BaseModel):
    b: str
    a: Optional[int] = None


def test_some_required():
    assert required_fields(A) == ["id", "title"]


def test_none_required():
    assert required_fields(B) == []


def test_sorted():
    assert required_fields(C) == ["b"]
