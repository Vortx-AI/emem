from pydantic import BaseModel


class Celsius(float):
    @classmethod
    def __get_validators__(cls):
        yield cls.validate

    @classmethod
    def validate(cls, v):
        try:
            f = float(v)
        except (TypeError, ValueError):
            raise ValueError("not a number")
        if f < -273.15:
            raise ValueError("below absolute zero")
        return cls(f)


class Reading(BaseModel):
    temp: Celsius
