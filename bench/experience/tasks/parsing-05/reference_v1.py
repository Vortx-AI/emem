from typing import List

from pydantic import BaseModel


class IdList(BaseModel):
    __root__: List[int]


def parse_ids(raw):
    return IdList.parse_raw(raw)


def ids_of(x):
    return list(x.__root__)
