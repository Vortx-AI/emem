Write `solution.py` (pydantic 1.10.x) defining a custom string type `HexColor` usable as a model field
type, and a model `Theme` with `primary: HexColor`. Accepted forms: `#rrggbb`, `rrggbb`, `#rgb`, `rgb`
(case-insensitive hex digits). The value is normalised to lowercase `#rrggbb` (3-digit forms are
expanded). Anything else raises `pydantic.ValidationError`.
