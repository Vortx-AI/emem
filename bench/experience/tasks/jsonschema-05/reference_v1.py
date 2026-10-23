def nested_model_names(model_cls):
    return sorted(model_cls.schema().get("definitions", {}))
