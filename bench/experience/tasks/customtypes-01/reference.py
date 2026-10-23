import re

from pydantic import BaseModel


class PostalCode(str):
    @classmethod
    def __get_validators__(cls):
        yield cls.validate

    @classmethod
    def validate(cls, v):
        if not isinstance(v, str) or not re.fullmatch(r"[0-9]{5}", v):
            raise ValueError("invalid postal code")
        return cls(v)


class Address(BaseModel):
    city: str
    zip: PostalCode
