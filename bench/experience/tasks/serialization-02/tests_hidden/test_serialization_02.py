import pytest
from pydantic import ValidationError
from solution import Account, to_wire


def test_roundtrip():
    a = Account(accountId="7", displayName="Zed")
    assert a.account_id == 7
    assert to_wire(a) == {"accountId": 7, "displayName": "Zed"}


def test_missing():
    with pytest.raises(ValidationError):
        Account(accountId=1)
