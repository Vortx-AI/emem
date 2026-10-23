Write `solution.py` (pydantic 1.10.x) defining `AppConfig` with `host: str` and `port: int = 8080`, and
`from_json(raw: str) -> AppConfig` that builds it from JSON text. Malformed JSON and invalid values both
raise `pydantic.ValidationError`.
