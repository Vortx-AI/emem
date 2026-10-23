from pydantic import BaseModel


class AppConfig(BaseModel):
    host: str
    port: int = 8080


def from_json(raw):
    return AppConfig.parse_raw(raw)
