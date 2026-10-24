#!/bin/bash
# Semantic retrieval with framed cards: each of the other two configurations
# lost one post-change task to one bad card, a different one each.
cd /home/ubuntu/emem-bench/bench/experience || exit 1
PY=/home/ubuntu/.emem-witness/venv/bin/python
for i in 1 2 3; do
  echo "$(date -u +%FT%TZ) pass $i semantic framed"
  $PY pilot.py --skip-publish --cards --k 2 --attempts 3 --framed
done
echo "$(date -u +%FT%TZ) DONE"
