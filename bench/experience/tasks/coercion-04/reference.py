from typing import Union

from pydantic import BaseModel


class Cell(BaseModel):
    value: Union[int, float, str]


def parse_cell(text):
    return Cell(value=text).value
