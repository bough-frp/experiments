#!/usr/bin/env bash
# Phase 6: every wall-clock bench and timed binary, in the handoff's order,
# on an idle machine. Each bench starts from an empty target/criterion, so
# no bench's samples mix with another's (groups `moves`, `build`, `nav`,
# `app` repeat across benches, and phase 5 left indicative samples there).
# After each bench, ratios.py runs over that bench's samples only, its
# output goes into the same result file, and the samples are archived.
#
# Writes results/, and archives Criterion data under
# ~/prog/bough/research-scratch-space/phase6-criterion/. Commits nothing.
#
# PHASE6_SMOKE=1 checks the plumbing in a few minutes: Criterion's --quick,
# the materializer's --quick, the two long timed binaries skipped, and
# everything written under research-scratch-space/phase6-smoke/ instead.

set -u
cd ~/prog/bough/experiments

DATE=$(date +%F)
COMMIT=$(git rev-parse --short HEAD)
RUSTC=$(rustc -V | sed -E 's/^rustc ([^ ]+) .*/\1/')
RELEASED=$(rustc -V | sed -E 's/.* ([0-9-]{10})\)$/\1/')
SMOKE=${PHASE6_SMOKE:-}
if [ -n "$SMOKE" ]; then
  ARCHIVE=~/prog/bough/research-scratch-space/phase6-smoke
  RESULTS=$ARCHIVE/results
  BENCH_ARGS=(-- --quick)
else
  ARCHIVE=~/prog/bough/research-scratch-space/phase6-criterion
  RESULTS=results
  BENCH_ARGS=()
fi
LOG=$ARCHIVE/run.log
# A fresh archive, so no mv below lands inside an old directory.
if [ -e "$ARCHIVE" ]; then
  echo "$ARCHIVE exists; move it aside first" >&2
  exit 1
fi
mkdir -p "$ARCHIVE" "$RESULTS"

log() { echo "$(date '+%F %T %z') $*" | tee -a "$LOG"; }
state() {
  echo "boost=$(cat /sys/devices/system/cpu/cpufreq/boost) governor=$(sort -u /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor | tr '\n' ' ')load=$(cut -d' ' -f1-3 /proc/loadavg)"
}

provenance() {
  echo "> rustc $RUSTC (released $RELEASED) - measured $DATE - $1 at experiments@$COMMIT - Ryzen 7 2700X, Fedora 44 container on Bazzite 44"
}

# Run a command, appending "$ cmd" and its output to a result file.
run_into() {
  local out=$1; shift
  { echo "\$ $*"; echo; } >> "$out"
  "$@" >> "$out" 2>&1
  local rc=$?
  echo >> "$out"
  return $rc
}

if [ -n "$(git status --porcelain)" ]; then
  echo "working tree not clean; refusing to run" >&2
  exit 1
fi

# Set aside whatever phase 5 left.
if [ -d target/criterion ]; then
  mv target/criterion "$ARCHIVE/pre-phase6"
fi

log "start${SMOKE:+ (smoke)} at experiments@$COMMIT; $(state); tuned profile latency-performance, set on the host"

BENCHES=(
  "rfd-0005-height-queue|"
  "rfd-0005-maintained-rank-queue|settled mixed churn lazy upkeep flat-settled flat-mixed flat-churn flat-lazy"
  "rfd-0005-heap-vs-mark-on-quiet-regions|ui-small ui-large frame"
  "rfd-0005-small-side-order|"
  "rfd-0005-bounded-relink-check|moves build"
  "rfd-0005-demand-bounded-push|nav app"
  "rfd-0003-work-paced-trigger|nav app pause-nav pause-app uneven-spread uneven-sparse pause-uneven-spread pause-uneven-sparse uneven-lagging fast-path"
  "rfd-0003-incremental-mark|incremental-"
  "rfd-0003-sweep-cost|live10 live90"
  "rfd-0003-rebrand-cost|"
  "rfd-0003-rebrand-write-cost|write_"
  "rfd-0004-erased-materializer|"
  "rfd-0004-patch-cell-crossover|vec- map- compose"
  "rfd-0006-lock-vs-queue-cost|single contended footprint-"
)

# Build everything first, so no compile heats the machine between benches
# and a build failure shows before an hour is spent.
log "building"
for entry in "${BENCHES[@]}"; do
  cargo bench --no-run --bench "${entry%%|*}-wallclock" >> "$LOG" 2>&1 \
    || { log "BUILD FAILED: ${entry%%|*}-wallclock"; exit 1; }
done
cargo build --release --bin rfd-0004-erased-materializer \
  --bin rfd-0002-decoupled-marker --bin rfd-0006-lock-vs-queue-cost >> "$LOG" 2>&1 \
  || { log "BUILD FAILED: binaries"; exit 1; }
log "built"

timed_bin() {  # timed_bin <name> <suffix> [args...]
  local name=$1 suffix=$2; shift 2
  local out=$RESULTS/$name-$suffix-$DATE.txt
  local t0=$SECONDS
  log "run $name $*; $(state)"
  provenance "$name" > "$out"
  run_into "$out" cargo run --release --bin "$name" -- "$@" \
    || log "FAILED: $name $*"
  log "done $name in $((SECONDS - t0)) s"
}

for entry in "${BENCHES[@]}"; do
  probe=${entry%%|*}
  prefixes=${entry#*|}
  target=$probe-wallclock
  out=$RESULTS/$target-$DATE.txt
  t0=$SECONDS
  log "bench $target; $(state)"
  provenance "$target" > "$out"
  if run_into "$out" cargo bench --bench "$target" "${BENCH_ARGS[@]}"; then
    # shellcheck disable=SC2086
    run_into "$out" python3 scripts/ratios.py $prefixes \
      || log "RATIOS FAILED: $target"
  else
    log "BENCH FAILED: $target"
  fi
  [ -d target/criterion ] && mv target/criterion "$ARCHIVE/$target"
  log "done $target in $((SECONDS - t0)) s"

  # Step 12's compile times follow its bench, as the handoff orders them.
  if [ "$probe" = rfd-0004-erased-materializer ]; then
    if [ -n "$SMOKE" ]; then
      timed_bin rfd-0004-erased-materializer compile-time --quick
    else
      timed_bin rfd-0004-erased-materializer compile-time
    fi
  fi
  # Step 14 sits between the patch-cell bench and the lock bench.
  if [ "$probe" = rfd-0004-patch-cell-crossover ] && [ -z "$SMOKE" ]; then
    timed_bin rfd-0002-decoupled-marker compile-time --compile-time
  fi
done
if [ -z "$SMOKE" ]; then
  timed_bin rfd-0006-lock-vs-queue-cost timed --timed
fi

log "end in $SECONDS s; $(state)"
