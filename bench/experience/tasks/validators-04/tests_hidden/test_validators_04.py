from solution import Article


def test_string_input():
    a = Article(title="t", tags=" Python, ,PyDantic ")
    assert a.tags == ["python", "pydantic"]


def test_list_input():
    assert Article(title="t", tags=["A", "b"]).tags == ["a", "b"]


def test_empty_string():
    assert Article(title="t", tags="").tags == []
