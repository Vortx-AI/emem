import pytest
from pydantic import ValidationError
from solution import AppConfig, from_json


def test_default_port():
    assert from_json('{"host":"h"}') == AppConfig(host="h", port=8080)


def test_coerced():
    assert from_json('{"host":"h","port":"81"}').port == 81


def test_malformed_json():
    with pytest.raises(ValidationError):
        from_json("{")


def test_invalid_value():
    with pytest.raises(ValidationError):
        from_json('{"host":"h","port":"x"}')
