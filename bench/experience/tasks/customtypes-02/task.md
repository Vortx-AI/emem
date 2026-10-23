Write `solution.py` (pydantic 1.10.x) defining a custom float type `Percent` (0 to 100 inclusive) usable
as a model field type, and a model `Discount` with `rate: Percent`. Accepts numbers and strings such as
`"50%"` or `"12.5"`. Anything else, or a value outside 0..100, is invalid
(`pydantic.ValidationError`).
