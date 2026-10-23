Write `solution.py` (pydantic 1.10.x) defining model `User` with `id: int`, `name: str`,
`email: Optional[str] = None`, `password: str`, and a function `to_payload(user) -> dict` that returns the
user as a plain dict without `password` and without any field whose value is None.
