#!/bin/sh
# Run the published Python reference natively on one dataset.
#   tools/ref.sh <a|b|c|d> <data dir>   ->  track  data  wall_s  peak_MB  sha256
# The sha256 is the expected answer for that data (the reference defines correct).
set -eu
track=$1 data=$2
case $track in a) mod=a_ingest out=summary.txt ;; b) mod=b_index out=results.txt ;;
               c) mod=c_join out=aggregates.txt ;; d) mod=d_traverse out=distances.txt ;;
               *) echo "track must be a, b, c or d" >&2; exit 2 ;; esac
root=$(cd "$(dirname "$0")/.." && pwd)
py=${PY:-$(uv python find 3.13)}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
start=$(python3 -c 'import time; print(time.monotonic())')
/usr/bin/time -l "$py" -I "$root/_spec/reference/$mod.py" --data "$data" --out "$tmp" 2> "$tmp/time.txt"
wall=$(python3 -c "import time; print(f'{time.monotonic() - $start:.2f}')")
rss=$(awk '/maximum resident set size/ {printf "%.0f", $1 / 1e6}' "$tmp/time.txt")
sha=$(shasum -a 256 "$tmp/$out" | cut -d' ' -f1)
echo "$track $data $wall $rss $sha"
