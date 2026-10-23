Write `solution.py` (pydantic 1.10.x) defining a model `User` with fields `name: str` and `age: int`.

- `name` is stripped of surrounding whitespace; if the stripped name is empty the model is invalid.
- `age` must be >= 0.

Invalid input must raise `pydantic.ValidationError`.
