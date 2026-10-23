Write `solution.py` targeting the installed pydantic (2.x). Define a model class `IdList` that
represents a bare JSON array of integers (the whole value is the list, not an object with a field),
`parse_ids(raw: str) -> IdList` that parses JSON text into it, and `ids_of(x) -> list` returning the
plain list of ints. Invalid content raises `pydantic.ValidationError`. `IdList` must be a
`pydantic.BaseModel` subclass.
