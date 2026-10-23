from typing import List

from pydantic import BaseModel, RootModel


class IdList(RootModel[List[int]]):
    pass


def parse_ids(raw):
    return IdList.model_validate_json(raw)


def ids_of(x):
    return list(x.root)
