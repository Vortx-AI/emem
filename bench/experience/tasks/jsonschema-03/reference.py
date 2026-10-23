def max_length_of(model_cls, field):
    return model_cls.schema()["properties"][field].get("maxLength")
