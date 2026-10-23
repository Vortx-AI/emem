import pytest
from pydantic import ValidationError
from solution import Contact


def test_strip():
    c = Contact(name="  a ", email=" a@b.c\n")
    assert (c.name, c.email) == ("a", "a@b.c")


def test_empty_after_strip():
    with pytest.raises(ValidationError):
        Contact(name="   ", email="a@b.c")


def test_no_per_field_validators():
    assert not Contact.__validators__
