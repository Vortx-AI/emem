Write `solution.py` (pydantic 1.10.x) defining `Contact` with `name: str` and `email: str`. All string
fields are stripped of surrounding whitespace automatically, and an empty string (after stripping) is
invalid. This must be configured on the model, not with per-field validators.
