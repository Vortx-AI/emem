import pytest
from pydantic import ValidationError
from solution import Rating


def test_none_allowed():
    assert Rating(item="a", score=None).score is None


def test_value():
    assert Rating(item="a", score="4").score == 4


def test_missing_score_invalid():
    with pytest.raises(ValidationError):
        Rating(item="a")
