from enum import Enum

from pydantic import BaseModel
from solution import enum_values


class Color(str, Enum):
    red = "red"
    green = "green"
    blue = "blue"


class Level(int, Enum):
    low = 1
    high = 2


class A(BaseModel):
    color: Color
    level: Level = Level.low


def test_no_default():
    assert enum_values(A, "color") == ["red", "green", "blue"]


def test_default():
    assert enum_values(A, "level") == [1, 2]
