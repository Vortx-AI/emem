Write `solution.py` (pydantic 1.10.x) defining `Item` with `sku: str` and `qty: int`, and
`update(item, **changes) -> Item`: a NEW validated item with the given field changes applied. The original
is not modified. Changes with invalid values raise `pydantic.ValidationError`.
