Write `solution.py` (pydantic 1.10.x) defining `Cart` with `count: int` and `label: str`, where NO
coercion is allowed: `count` accepts only real ints (not strings, floats or bools) and `label` accepts only
real strings (not numbers). Violations raise `pydantic.ValidationError`.
