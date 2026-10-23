import pytest
from pydantic import ValidationError
from solution import User


def test_strip():
    assert User(name="  bob ", age=3).name == "bob"


def test_blank_name():
    with pytest.raises(ValidationError):
        User(name="   ", age=3)


def test_negative_age():
    with pytest.raises(ValidationError):
        User(name="a", age=-1)


def test_zero_age_ok():
    assert User(name="a", age=0).age == 0
