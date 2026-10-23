Write `solution.py` (pydantic 1.10.x) defining `Account` with fields `account_id: int` (wire name
`accountId`) and `display_name: str` (wire name `displayName`). It is built from the camelCase wire names.
Define `to_wire(acct) -> dict` returning the camelCase dict.
