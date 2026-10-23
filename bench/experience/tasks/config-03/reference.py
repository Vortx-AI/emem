from pydantic import BaseModel, Extra, Field


class Settings(BaseModel):
    api_key: str = Field(alias="apiKey")
    timeout: int = 30

    class Config:
        extra = Extra.forbid
        allow_population_by_field_name = True
