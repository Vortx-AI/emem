from solution import User, to_payload


def test_drops_password_and_none():
    assert to_payload(User(id=1, name="a", password="x")) == {"id": 1, "name": "a"}


def test_keeps_email():
    p = to_payload(User(id=1, name="a", email="e@x.io", password="x"))
    assert p == {"id": 1, "name": "a", "email": "e@x.io"}


def test_returns_plain_dict():
    assert type(to_payload(User(id=1, name="a", password="x"))) is dict
