Write `solution.py` (pydantic 1.10.x) defining `enum_values(model_cls, field) -> list`: the allowed
values of an Enum-typed field, in definition order, read from the model's JSON schema (not by importing the
Enum). It must work for fields with and without a default.
