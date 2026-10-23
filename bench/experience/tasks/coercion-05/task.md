Write `solution.py` targeting the installed pydantic (2.x) defining `Device` with `id: str`. Legacy
payloads send the id as a number, so `Device(id=42).id` must equal `"42"`. Strings stay as they are.
Anything that is neither a string nor an int/float (such as a list or None) raises
`pydantic.ValidationError`.
