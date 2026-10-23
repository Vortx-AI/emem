from pydantic import BaseModel


class Contact(BaseModel):
    name: str
    email: str

    class Config:
        anystr_strip_whitespace = True
        min_anystr_length = 1
