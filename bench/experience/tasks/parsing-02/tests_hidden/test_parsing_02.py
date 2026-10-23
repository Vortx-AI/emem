import pytest
from pydantic import ValidationError
from solution import Item, load_many


def test_many():
    out = load_many('[{"sku":"a","qty":"2"},{"sku":"b","qty":5}]')
    assert [type(i) for i in out] == [Item, Item]
    assert [i.qty for i in out] == [2, 5]


def test_empty():
    assert load_many("[]") == []


def test_bad_element():
    with pytest.raises(ValidationError):
        load_many('[{"sku":"a"}]')
