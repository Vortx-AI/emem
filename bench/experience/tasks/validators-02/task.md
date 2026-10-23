Write `solution.py` (pydantic 1.10.x) defining a model `Signup` with `password: str` and `confirm: str`.

- `password` must be at least 8 characters.
- `confirm` must equal `password`. The error for a mismatch must be attached to the `confirm` field.

Invalid input must raise `pydantic.ValidationError`.
