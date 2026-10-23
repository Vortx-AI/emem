def field_titles(model_cls):
    props = model_cls.schema()["properties"]
    return {name: p["title"] for name, p in props.items()}
