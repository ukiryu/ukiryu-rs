#!/usr/bin/env bash
# Run the Rust ukiryu CLI checks inside each supported distro container.
# Port of the Ruby repository's docker-test-all.sh: same five platforms, one
# image per platform, the register mounted read-only, one summary at the end.
# The Rust CLI is a prebuilt binary (never built here), mounted into the
# container, so the container needs no Ruby and no toolchain.
#
# Usage: scripts/docker-test-all.sh [--binary-gnu PATH] [--binary-musl PATH]
#        [--register PATH] [--build] [--verbose] [platform ...]
# Platforms: alpine ubuntu-22 ubuntu-24 debian-bookworm debian-trixie
# The musl binary is used for alpine, the gnu binary for the other four.
# Exit status is non-zero if any selected platform fails.
# Do not use 'set -e': every platform runs even if an earlier one fails.
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLATFORMS=(alpine ubuntu-22 ubuntu-24 debian-bookworm debian-trixie)
declare -A SUFFIX=([alpine]=alpine [ubuntu-22]=ubuntu-22.04 [ubuntu-24]=ubuntu-24.04
                   [debian-bookworm]=debian-bookworm [debian-trixie]=debian-trixie)
BIN_GNU="" BIN_MUSL="" REGISTER="" BUILD=false VERBOSE=false SELECTED=()

while [ $# -gt 0 ]; do
  case $1 in
    --binary-gnu) BIN_GNU="$2"; shift 2 ;;
    --binary-musl) BIN_MUSL="$2"; shift 2 ;;
    --register|-r) REGISTER="$2"; shift 2 ;;
    --build|-b) BUILD=true; shift ;;
    --verbose|-v) VERBOSE=true; shift ;;
    --help|-h) sed -n '2,13p' "$0"; exit 0 ;;
    alpine|ubuntu-22|ubuntu-24|debian-bookworm|debian-trixie) SELECTED+=("$1"); shift ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done
[ ${#SELECTED[@]} -gt 0 ] && PLATFORMS=("${SELECTED[@]}")

if ! command -v docker >/dev/null || ! docker info >/dev/null 2>&1; then
  echo "docker is not available" >&2; exit 2
fi
if [ -z "$REGISTER" ] || [ ! -d "$REGISTER" ]; then
  echo "--register PATH must name an existing register checkout" >&2; exit 2
fi
LOGDIR="${LOGDIR:-$ROOT/docker-test-logs}"
mkdir -p "$LOGDIR"
declare -A RESULT

run_platform() {
  local p=$1 suffix=${SUFFIX[$1]} bin image
  image="ukiryu-rs-test:$suffix"
  if [ "$p" = alpine ]; then bin=$BIN_MUSL; else bin=$BIN_GNU; fi
  if [ -z "$bin" ] || [ ! -x "$bin" ]; then
    echo "[$p] missing executable binary: '$bin'"; RESULT[$p]="FAILED (no binary)"; return 1
  fi
  echo "== [$p] building image $image"
  if ! docker build -f "$ROOT/docker/Dockerfile.$suffix" -t "$image" "$ROOT/docker" \
        >"$LOGDIR/build-$p.log" 2>&1; then
    tail -n 40 "$LOGDIR/build-$p.log"; RESULT[$p]="FAILED (image build)"; return 1
  fi
  echo "== [$p] running checks with $(basename "$bin")"
  # Repository and register are read-only; the checks copy fixtures to a
  # writable temp dir inside the container.
  if docker run --rm \
      -v "$ROOT:/ukiryu-rs:ro" -v "$REGISTER:/register:ro" \
      -v "$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin"):/usr/local/bin/ukiryu:ro" \
      -e UKIRYU_REGISTER=/register -e PLATFORM="$p" -e VERBOSE="$VERBOSE" \
      "$image" bash /ukiryu-rs/scripts/container-checks.sh >"$LOGDIR/run-$p.log" 2>&1; then
    RESULT[$p]=PASSED
  else
    RESULT[$p]=FAILED
  fi
  cat "$LOGDIR/run-$p.log"
}

for p in "${PLATFORMS[@]}"; do run_platform "$p"; done

echo; echo "== Summary"
failed=0
for p in "${PLATFORMS[@]}"; do
  echo "$p: ${RESULT[$p]:-SKIPPED}"
  [ "${RESULT[$p]:-}" = PASSED ] || failed=1
done
exit $failed
