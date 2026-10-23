import re

from pydantic import BaseModel


class Slug(str):
    @classmethod
    def __get_validators__(cls):
        yield cls.validate

    @classmethod
    def validate(cls, v):
        if not isinstance(v, str):
            raise TypeError("string required")
        s = re.sub(r" +", "-", v.strip().lower())
        if not re.fullmatch(r"[a-z0-9]+(-[a-z0-9]+)*", s):
            raise ValueError("invalid slug")
        return cls(s)


class Page(BaseModel):
    slug: Slug
