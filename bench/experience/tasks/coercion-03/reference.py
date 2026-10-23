from pydantic import BaseModel, conint, constr


class Code(BaseModel):
    code: constr(regex=r"^[A-Z]{3}[0-9]{2}$")
    pct: conint(ge=0, le=100)
