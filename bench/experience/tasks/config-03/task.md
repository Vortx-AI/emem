Write `solution.py` (pydantic 1.10.x) defining `Settings` with `api_key: str` (wire name `apiKey`) and
`timeout: int = 30`. It can be built with either the wire name or the field name. Unknown keys are
rejected with `pydantic.ValidationError`.
