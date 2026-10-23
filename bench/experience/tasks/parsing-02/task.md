Write `solution.py` (pydantic 1.10.x) defining `Item` with `sku: str` and `qty: int`, and
`load_many(raw: str) -> list` that parses a JSON array of objects (given as text) into a list of `Item`.
Invalid content raises `pydantic.ValidationError`.
