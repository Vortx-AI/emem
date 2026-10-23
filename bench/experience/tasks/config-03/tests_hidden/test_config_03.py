import pytest
from pydantic import ValidationError
from solution import Settings


def test_alias():
    assert Settings(apiKey="k").api_key == "k"


def test_field_name():
    s = Settings(api_key="k", timeout="5")
    assert (s.api_key, s.timeout) == ("k", 5)


def test_extra_rejected():
    with pytest.raises(ValidationError):
        Settings(api_key="k", other=1)
