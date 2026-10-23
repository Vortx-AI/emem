Write `solution.py` (pydantic 1.10.x) defining `Filters` with optional fields `tags: List[str]`,
`limit: int` and `q: str`. Any of them may be left out, and a missing one means None. Also define
`active(f) -> dict` returning only the filters that are set (not None).
