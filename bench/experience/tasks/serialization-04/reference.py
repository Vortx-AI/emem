from pydantic import BaseModel


class Customer(BaseModel):
    name: str
    ssn: str


class Order(BaseModel):
    id: int
    customer: Customer
    notes: str


def public_view(order):
    return order.dict(exclude={"notes": ..., "customer": {"ssn"}})
