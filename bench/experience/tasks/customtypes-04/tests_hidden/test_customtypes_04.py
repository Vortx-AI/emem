import pytest
from pydantic import ValidationError
from solution import Page


def test_normalise():
    assert Page(slug="  Hello   World ").slug == "hello-world"
    assert Page(slug="a-b").slug == "a-b"


@pytest.mark.parametrize("bad", ["", "a_b", "--a", "a--b", "a!", 5])
def test_bad(bad):
    with pytest.raises(ValidationError):
        Page(slug=bad)
