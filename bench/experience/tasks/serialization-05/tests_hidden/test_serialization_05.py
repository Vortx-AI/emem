from solution import Event, to_canonical_json


def test_canonical():
    ev = Event(name="x", at="2024-01-02T03:04:05", tags=["a", "b"])
    assert to_canonical_json(ev) == '{"at":"2024-01-02T03:04:05","name":"x","tags":["a","b"]}'


def test_empty_tags():
    ev = Event(name="y", at="2020-05-06T00:00:00", tags=[])
    assert to_canonical_json(ev) == '{"at":"2020-05-06T00:00:00","name":"y","tags":[]}'
