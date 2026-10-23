Write `solution.py` (pydantic 1.10.x) defining `UserOut` with `id: int` and `name: str`, and
`to_out(obj) -> UserOut` that builds a `UserOut` from any plain Python object having `id` and `name`
attributes (extra attributes are ignored). A missing attribute must raise `pydantic.ValidationError`.
