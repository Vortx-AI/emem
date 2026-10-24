#!/bin/bash
# Three passes of each card configuration, to measure run-to-run noise
# before reading any difference between them.
cd /home/ubuntu/emem-bench/bench/experience || exit 1
PY=/home/ubuntu/.emem-witness/venv/bin/python
for i in 1 2 3; do
  echo "$(date -u +%FT%TZ) pass $i semantic"
  $PY pilot.py --skip-publish --cards --k 2 --attempts 3
  echo "$(date -u +%FT%TZ) pass $i lexical framed"
  $PY pilot.py --skip-publish --cards --k 2 --attempts 3 --mode lexical --framed
done
echo "$(date -u +%FT%TZ) DONE"
