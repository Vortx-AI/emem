from pydantic import BaseModel


class UserOut(BaseModel):
    id: int
    name: str

    class Config:
        orm_mode = True


def to_out(obj):
    return UserOut.from_orm(obj)
