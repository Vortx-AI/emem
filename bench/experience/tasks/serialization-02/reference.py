from pydantic import BaseModel, Field


class Account(BaseModel):
    account_id: int = Field(alias="accountId")
    display_name: str = Field(alias="displayName")


def to_wire(acct):
    return acct.dict(by_alias=True)
