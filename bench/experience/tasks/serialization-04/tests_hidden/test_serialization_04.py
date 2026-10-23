from solution import Customer, Order, public_view


def test_view():
    o = Order(id=1, customer=Customer(name="a", ssn="123"), notes="vip")
    assert public_view(o) == {"id": 1, "customer": {"name": "a"}}


def test_nested_is_dict():
    o = Order(id=2, customer={"name": "b", "ssn": "9"}, notes="")
    assert type(public_view(o)["customer"]) is dict
