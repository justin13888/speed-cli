#!/usr/bin/env bash
# Loopback benchmark: build, start `speed-cli server --all` on 127.0.0.1,
# run `speed-cli suite` against it, and save the CBOR report to target/.
#
# The loopback path measures the software stack (CPU-bound), not a network —
# use it for A/B comparisons of build configurations, e.g. allocators:
#
#   LABEL=mimalloc  ./scripts/loopback-bench.sh
#   LABEL=sysmalloc ./scripts/loopback-bench.sh --no-default-features
#
# Env:  DURATION      per-phase seconds (default 30)
#       CONTROL_PORT  control endpoint port (default 9123, off the usual 9000)
#       LABEL         tag embedded in the output filename (default "run")
# Args: passed through to `cargo build`.
#
# Unix-only (uses /dev/tcp for the readiness probe). See docs/PROFILING.md
# for methodology (interleaved runs, medians, idle machine).
set -euo pipefail
cd "$(dirname "$0")/.."

DURATION="${DURATION:-30}"
CONTROL_PORT="${CONTROL_PORT:-9123}"
LABEL="${LABEL:-run}"

cargo build --release "$@"
BIN=target/release/speed-cli

"$BIN" -q server --all -b 127.0.0.1 --control-port "$CONTROL_PORT" &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true; wait "$SERVER_PID" 2>/dev/null || true' EXIT

# Wait (max ~10s) for the control endpoint to accept connections.
ready=0
for _ in $(seq 1 50); do
  if (exec 3<>"/dev/tcp/127.0.0.1/${CONTROL_PORT}") 2>/dev/null; then
    exec 3>&- || true
    ready=1
    break
  fi
  sleep 0.2
done
if [ "$ready" -ne 1 ]; then
  echo "error: server did not open control port ${CONTROL_PORT} within 10s" >&2
  exit 1
fi

OUT="target/loopback-${LABEL}-$(date +%Y%m%d-%H%M%S).cbor"
"$BIN" suite -s 127.0.0.1 --control-port "$CONTROL_PORT" -d "$DURATION" -e "$OUT"
echo
echo "Report saved to ${OUT}"
echo "Render it with: ${BIN} report -f ${OUT} [--export-html report.html]"
