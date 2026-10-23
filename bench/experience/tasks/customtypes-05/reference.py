from typing import Any

from pydantic import BaseModel, GetCoreSchemaHandler
from pydantic_core import core_schema


class Celsius(float):
    @classmethod
    def __get_pydantic_core_schema__(cls, source: Any, handler: GetCoreSchemaHandler):
        return core_schema.no_info_plain_validator_function(cls._validate)

    @classmethod
    def _validate(cls, v):
        try:
            f = float(v)
        except (TypeError, ValueError):
            raise ValueError("not a number")
        if f < -273.15:
            raise ValueError("below absolute zero")
        return cls(f)


class Reading(BaseModel):
    temp: Celsius
