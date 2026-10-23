Write `solution.py` targeting the installed pydantic (2.x) defining a custom float type `Celsius`
usable as a model field type, and a model `Reading` with `temp: Celsius`. Accepts numbers and numeric
strings (`"21.5"`). Values below absolute zero (-273.15) and non-numeric input raise
`pydantic.ValidationError`. The validated value is a `Celsius` instance.
