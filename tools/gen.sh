#!/bin/sh
# Generate one dataset with the published generator inside the reference's pinned image
# (byte-identical on any machine, and fast even when the host shell runs at low priority).
#   tools/gen.sh <a|b|c|d> <seed> [scale=full]   ->  .data/<track>/<scale>-seed<seed>
set -eu
track=$1 seed=$2 scale=${3:-full}
case $track in a) gen=a_ingest ;; b) gen=b_index ;; c) gen=c_join ;; d) gen=d_traverse ;;
               *) echo "track must be a, b, c or d" >&2; exit 2 ;; esac
root=$(cd "$(dirname "$0")/.." && pwd)
image=python:3.13-slim@sha256:d98cc29d3266e535dd9da1099688d9cb3ca4753813571e071763d17725460962
docker run --rm --network=none -v "$root:/w" -w /w "$image" \
  python3 -I "_spec/generators/$gen.py" --seed "$seed" --scale "$scale" --out ".data/$track/$scale-seed$seed"
