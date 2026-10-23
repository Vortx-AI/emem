import pytest
from solution import Counter, bump


def test_immutable():
    c = Counter(n=1)
    with pytest.raises((TypeError, ValueError)):
        c.n = 2
    assert c.n == 1


def test_bump():
    c = Counter()
    d = bump(c)
    assert (c.n, d.n) == (0, 1)
