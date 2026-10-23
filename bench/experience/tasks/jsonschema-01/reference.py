def required_fields(model_cls):
    return sorted(model_cls.schema().get("required", []))
