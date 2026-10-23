Write `solution.py` (pydantic 1.10.x) defining `required_fields(model_cls) -> list` that returns the
sorted names of the fields a caller must supply, according to the model's JSON schema. Fields with
defaults are not required.
