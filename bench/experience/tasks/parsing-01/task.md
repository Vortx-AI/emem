Write `solution.py` (pydantic 1.10.x) defining `Item` with `sku: str` and `qty: int`, and
`load(data: dict) -> Item` that validates a plain dict into an `Item` (string numbers are coerced).
Invalid data raises `pydantic.ValidationError`.
