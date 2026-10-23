Write `solution.py` (pydantic 1.10.x) defining `Address(city: str, zip: Optional[str])` and
`User(name: str, address: Optional[Address])`, and `city_of(data: dict) -> Optional[str]` that validates
`data` as a User and returns the address city, or None when the user has no address. Invalid data raises
`pydantic.ValidationError`; a user without an address is valid.
