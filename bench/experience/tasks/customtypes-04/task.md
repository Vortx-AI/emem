Write `solution.py` (pydantic 1.10.x) defining a custom string type `Slug` usable as a model field type,
and a model `Page` with `slug: Slug`. The input is stripped, lowercased and inner runs of spaces become one
hyphen. The result must then match lowercase letters/digits separated by single hyphens
(`^[a-z0-9]+(-[a-z0-9]+)*$`), otherwise `pydantic.ValidationError`.
