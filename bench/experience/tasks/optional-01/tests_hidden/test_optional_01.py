import pytest
from pydantic import ValidationError
from solution import Profile


def test_absent_nickname():
    assert Profile(name="a").nickname is None


def test_given_nickname():
    assert Profile(name="a", nickname="b").nickname == "b"


def test_name_required():
    with pytest.raises(ValidationError):
        Profile()
