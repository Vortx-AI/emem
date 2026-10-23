from pydantic import BaseModel, Field
from solution import field_titles


class A(BaseModel):
    first_name: str
    age: int = Field(1, title="Years")


def test_titles():
    assert field_titles(A) == {"first_name": "First Name", "age": "Years"}
