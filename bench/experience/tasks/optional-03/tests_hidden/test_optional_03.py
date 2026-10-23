from solution import Filters, active


def test_none_set():
    assert active(Filters()) == {}


def test_some_set():
    assert active(Filters(limit="5", q="x")) == {"limit": 5, "q": "x"}


def test_tags():
    assert active(Filters(tags=["a"])) == {"tags": ["a"]}
