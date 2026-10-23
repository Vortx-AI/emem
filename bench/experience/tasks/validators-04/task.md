Write `solution.py` (pydantic 1.10.x) defining `Article` with `title: str` and `tags: List[str]`.

- `tags` may be given as a list OR as one comma-separated string such as `" Python, ,PyDantic "`.
  A string is split on commas, each piece stripped, empty pieces dropped.
- Every tag ends up lowercase.
