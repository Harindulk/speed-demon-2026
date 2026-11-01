#!/bin/sh
# Check a track against every local dataset whose expected hash is in tools/expected.txt.
#   tools/verify.sh <a|b|c|d>      (needs Docker; data from tools/gen.sh)
# Builds once with check.py (board sandbox settings), then runs each seed with --expected.
set -eu
track=$1
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
first=1 fails=0
while read -r t seed sha; do
  [ "$t" = "$track" ] || continue
  data=.data/$t/full-seed$seed
  [ -d "$data" ] || { echo "skip  seed $seed: no $data (tools/gen.sh $t $seed)"; continue; }
  if [ $first = 1 ]; then flag=; first=0; else flag=--skip-build; fi
  if out=$(python3 check.py "$t" --data "$data" --expected "$sha" $flag 2>&1); then
    echo "PASS  seed $seed  $(echo "$out" | awk '/^time/ {print $2 " s"}')"
  else
    echo "FAIL  seed $seed"; echo "$out" | tail -3; fails=$((fails + 1))
  fi
done < tools/expected.txt
[ $fails = 0 ]
