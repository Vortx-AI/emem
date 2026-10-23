def enum_values(model_cls, field):
    schema = model_cls.schema()
    prop = schema["properties"][field]
    ref = prop.get("$ref") or prop["allOf"][0]["$ref"]
    return list(schema["definitions"][ref.rsplit("/", 1)[-1]]["enum"])
