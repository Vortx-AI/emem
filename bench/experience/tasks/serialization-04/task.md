Write `solution.py` (pydantic 1.10.x) defining `Customer(name: str, ssn: str)` and
`Order(id: int, customer: Customer, notes: str)`, and `public_view(order) -> dict`: the order as a plain
nested dict without `notes` and without the customer's `ssn`.
