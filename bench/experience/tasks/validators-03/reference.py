from datetime import date

from pydantic import BaseModel, validator


class Booking(BaseModel):
    start: date
    end: date

    @validator("end")
    def _order(cls, v, values):
        if "start" in values and v < values["start"]:
            raise ValueError("end before start")
        return v


def nights(b):
    return (b.end - b.start).days
