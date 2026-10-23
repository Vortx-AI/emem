from pydantic import BaseModel, ConfigDict


class Device(BaseModel):
    model_config = ConfigDict(coerce_numbers_to_str=True)

    id: str
