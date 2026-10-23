import pytest
from pydantic import ValidationError
from solution import Row, parse_row


def test_csv_like():
    r = parse_row({"id": "7", "price": "3.5", "active": "yes"})
    assert (r.id, r.price, r.active) == (7, 3.5, True)


def test_false_words():
    assert parse_row({"id": 1, "price": 1, "active": "off"}).active is False
    assert parse_row({"id": 1, "price": 1, "active": "0"}).active is False


def test_bad_bool():
    with pytest.raises(ValidationError):
        parse_row({"id": 1, "price": 1, "active": "maybe"})
