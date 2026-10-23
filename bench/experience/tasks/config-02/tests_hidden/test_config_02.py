import pytest
from solution import Point


def test_immutable():
    p = Point(x=1, y=2)
    with pytest.raises((TypeError, ValueError)):
        p.x = 5
    assert p.x == 1


def test_hashable():
    assert hash(Point(x=1, y=2)) == hash(Point(x=1, y=2))
    assert len({Point(x=1, y=2), Point(x=1, y=2), Point(x=2, y=2)}) == 2
