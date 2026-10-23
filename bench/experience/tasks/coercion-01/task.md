Write `solution.py` (pydantic 1.10.x) defining `Row` with `id: int`, `price: float`, `active: bool`, and
`parse_row(d: dict) -> Row` that builds a Row from loosely-typed input (such as CSV text:
`{"id": "7", "price": "3.5", "active": "yes"}`). Truthy/falsy words follow pydantic's usual bool parsing.
Unparseable values raise `pydantic.ValidationError`.
