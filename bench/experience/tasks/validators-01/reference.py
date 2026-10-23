from pydantic import BaseModel, validator


class User(BaseModel):
    name: str
    age: int

    @validator("name")
    def _name(cls, v):
        v = v.strip()
        if not v:
            raise ValueError("name must not be blank")
        return v

    @validator("age")
    def _age(cls, v):
        if v < 0:
            raise ValueError("age must be >= 0")
        return v
