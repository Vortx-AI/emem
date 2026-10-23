import pytest
from pydantic import ValidationError
from solution import Theme


@pytest.mark.parametrize(
    "given,want",
    [("ABC", "#aabbcc"), ("#A1B2C3", "#a1b2c3"), ("a1b2c3", "#a1b2c3"), ("#fff", "#ffffff")],
)
def test_normalised(given, want):
    assert Theme(primary=given).primary == want


@pytest.mark.parametrize("bad", ["xyz", "#12345", "", 123456, "#1234567"])
def test_bad(bad):
    with pytest.raises(ValidationError):
        Theme(primary=bad)
