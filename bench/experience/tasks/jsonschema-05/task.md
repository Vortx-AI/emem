Write `solution.py` targeting the installed pydantic (2.x) defining `nested_model_names(model_cls) ->
list`: the sorted names of every sub-model the model's JSON schema defines for reuse (models nested inside
fields, lists, and so on). A model with no nested models gives an empty list.
