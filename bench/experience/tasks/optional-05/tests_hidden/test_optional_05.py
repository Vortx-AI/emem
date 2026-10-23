import pytest
from pydantic import ValidationError
from solution import Person


def test_no_middle():
    assert Person(first="a", last="b").middle is None


def test_middle():
    assert Person(first="a", last="b", middle="c").middle == "c"


def test_required():
    with pytest.raises(ValidationError):
        Person(first="a")
