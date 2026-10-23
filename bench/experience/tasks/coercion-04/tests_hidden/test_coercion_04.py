from solution import Cell, parse_cell


def test_int():
    v = parse_cell("5")
    assert v == 5 and type(v) is int
    assert parse_cell("-3") == -3


def test_float():
    v = parse_cell("5.5")
    assert v == 5.5 and type(v) is float


def test_string():
    assert parse_cell("abc") == "abc"


def test_uses_model():
    assert "value" in Cell.__fields__
