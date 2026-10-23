from pydantic import BaseModel, validator


class Signup(BaseModel):
    password: str
    confirm: str

    @validator("password")
    def _len(cls, v):
        if len(v) < 8:
            raise ValueError("password too short")
        return v

    @validator("confirm")
    def _match(cls, v, values):
        if "password" in values and v != values["password"]:
            raise ValueError("passwords do not match")
        return v
