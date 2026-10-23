def nested_model_names(model_cls):
    return sorted(model_cls.model_json_schema().get("$defs", {}))
