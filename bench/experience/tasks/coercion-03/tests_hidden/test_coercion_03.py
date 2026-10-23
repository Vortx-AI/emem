import pytest
from pydantic import ValidationError
from solution import Code


def test_ok():
    c = Code(code="ABC12", pct="50")
    assert (c.code, c.pct) == ("ABC12", 50)


@pytest.mark.parametrize("code", ["abc12", "ABCD1", "AB123", "ABC12 "])
def test_bad_code(code):
    with pytest.raises(ValidationError):
        Code(code=code, pct=1)


@pytest.mark.parametrize("pct", [-1, 101, "x"])
def test_bad_pct(pct):
    with pytest.raises(ValidationError):
        Code(code="ABC12", pct=pct)
