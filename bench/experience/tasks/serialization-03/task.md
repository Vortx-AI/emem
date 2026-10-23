Write `solution.py` (pydantic 1.10.x) defining `Patch` with optional fields `name: Optional[str]`,
`age: Optional[int]`, `nickname: Optional[str]`, all defaulting to None, and `patch_body(p) -> dict`
containing only the fields the caller explicitly supplied. An explicitly supplied None is kept.
