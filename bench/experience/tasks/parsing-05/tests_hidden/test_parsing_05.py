import pytest
from pydantic import BaseModel, ValidationError
from solution import IdList, ids_of, parse_ids


def test_parse():
    x = parse_ids("[1, \"2\", 3]")
    assert isinstance(x, IdList) and issubclass(IdList, BaseModel)
    assert ids_of(x) == [1, 2, 3]


def test_empty():
    assert ids_of(parse_ids("[]")) == []


def test_invalid():
    with pytest.raises(ValidationError):
        parse_ids('["a"]')
