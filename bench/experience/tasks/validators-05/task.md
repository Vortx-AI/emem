Write `solution.py` targeting the installed pydantic (2.x) defining `Order` with `items: List[float]`
(item prices) and `total: float`. The model is valid only when `total` equals the sum of `items`
(tolerance 1e-9). The check must run on the whole model, after field validation. Invalid input, including
an `items` list containing a non-number, must raise `pydantic.ValidationError` and nothing else.
