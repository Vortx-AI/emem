import re

from pydantic import BaseModel


class HexColor(str):
    @classmethod
    def __get_validators__(cls):
        yield cls.validate

    @classmethod
    def validate(cls, v):
        if not isinstance(v, str):
            raise TypeError("string required")
        s = v.lstrip("#").lower()
        if not re.fullmatch(r"[0-9a-f]{3}|[0-9a-f]{6}", s):
            raise ValueError("invalid colour")
        if len(s) == 3:
            s = "".join(c * 2 for c in s)
        return cls("#" + s)


class Theme(BaseModel):
    primary: HexColor
