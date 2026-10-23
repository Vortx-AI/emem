Write `solution.py` (pydantic 1.10.x) defining `parse_cell(text: str)` that returns an `int` when the text
is an integer (`"5"`, `"-3"`), a `float` when it is a decimal number (`"5.5"`), and otherwise the string
unchanged. It must be built on a pydantic model with a union-typed field named `value`.
