Write `solution.py` (pydantic 1.10.x) defining a model `Booking` with `start: date` and `end: date`
(`datetime.date`), where `end` must not be earlier than `start` (same day is fine). Also define
`nights(b) -> int`, the number of nights between start and end.

Invalid input must raise `pydantic.ValidationError` and the error must be reported on `end`.
