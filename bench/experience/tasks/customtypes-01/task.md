Write `solution.py` (pydantic 1.10.x) defining a custom string type `PostalCode` (5 ASCII digits,
anything else invalid) usable as a model field type, and a model `Address` with `city: str` and
`zip: PostalCode`. Invalid input raises `pydantic.ValidationError`.
