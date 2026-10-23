from pydantic import BaseModel


class Percent(float):
    @classmethod
    def __get_validators__(cls):
        yield cls.validate

    @classmethod
    def validate(cls, v):
        if isinstance(v, str):
            v = v.strip().rstrip("%")
        try:
            f = float(v)
        except (TypeError, ValueError):
            raise ValueError("not a percentage")
        if not 0 <= f <= 100:
            raise ValueError("out of range")
        return cls(f)


class Discount(BaseModel):
    rate: Percent
