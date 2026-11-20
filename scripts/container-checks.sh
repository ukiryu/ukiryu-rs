#!/usr/bin/env bash
# Checks run inside one distro container against the mounted `ukiryu` binary.
# Required: /usr/local/bin/ukiryu, /register (read-only), /ukiryu-rs (read-only).
#
# Result classes:
#   PASS     check ran and held
#   FAIL     check ran and did not hold (the run fails)
#   PENDING  verb exits 2 (GATED_EXIT): its handler is not merged yet. Each
#            pending check names the open PR that will land it. Once a PR is
#            merged the verb stops exiting 2 and the same row must then pass,
#            so no edit is needed to switch a row on.
set -u
export NO_COLOR=1
FIXTURES=/ukiryu-rs/ukiryu-cli/tests
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
export HOME="$W/home"; mkdir -p "$HOME"
cp -R "$FIXTURES/execute_data/fixture" "$W/exec"
cp -R "$FIXTURES/author_data/tree" "$W/author"
pass=0 fail=0 pending=0
OUT="" ERR="" CODE=0

run() { # run ARGS... ; captures OUT ERR CODE (cwd and env from caller)
  OUT=$(ukiryu "$@" 2>"$W/err"); CODE=$?; ERR=$(cat "$W/err")
}
ok()   { pass=$((pass+1)); echo "PASS    $1"; }
bad()  { fail=$((fail+1)); echo "FAIL    $1"; echo "        exit=$CODE"; echo "        stdout: ${OUT:0:300}"; echo "        stderr: ${ERR:0:300}"; }
pend() { pending=$((pending+1)); echo "PENDING $1 (waits for $2)"; }
check() { # check NAME WANT_CODE STDOUT_SUBSTR STDERR_SUBSTR
  local name=$1 want=$2 so=$3 se=$4
  if [ "$CODE" = "$want" ] && [[ "$OUT" == *"$so"* ]] && [[ "$ERR" == *"$se"* ]]; then ok "$name"; else bad "$name"; fi
}
# A verb whose handler may not be merged yet: exit 2 means PENDING.
verb() { # verb NAME PR ARGS...
  local name=$1 pr=$2; shift 2
  run "$@"
  if [ "$CODE" = 2 ]; then pend "$name" "$pr"
  elif [ "$CODE" = 0 ] && [ -n "$OUT" ]; then ok "$name"
  else bad "$name"; fi
}

echo "platform: ${PLATFORM:-?}  $(. /etc/os-release && echo "$PRETTY_NAME")  arch: $(uname -m)  libc: $( [ -e /lib/ld-musl-$(uname -m).so.1 ] && echo musl || echo glibc)"
file -L /usr/local/bin/ukiryu 2>/dev/null | head -1

# --- binary and mounts
[ -x /usr/local/bin/ukiryu ] && ok "binary is executable" || { CODE=1; bad "binary is executable"; }
[ -d /register/tools ] && ok "register mounted with tools/" || { CODE=1; bad "register mounted with tools/"; }
if touch /register/.write-probe 2>/dev/null; then rm -f /register/.write-probe; CODE=1; bad "register is read-only"; else ok "register is read-only"; fi
if [ -d /register/tools/jq ]; then ok "register holds the jq tool"; else CODE=1; bad "register holds the jq tool"; fi

# --- verbs merged on main
run version;            check "version" 0 "Ukiryu" ""
run --help;             check "--help lists exec" 0 "exec" ""
run;                    check "no verb prints usage" 0 "exec" ""
export SHELL=/bin/bash
run system;             check "system (SHELL=bash)" 0 "Available shells" ""
for sh in bash zsh fish dash tcsh; do
  path=$(command -v "$sh") || { CODE=1; bad "shell $sh installed"; continue; }
  SHELL=$path run system; check "system detects SHELL=$sh" 0 "Available shells" ""
done
run config set format json;   check "config set" 0 "Set format = json" ""
run config get format;        check "config get" 0 "format:" ""
run config unset format;      check "config unset" 0 "Unset format" ""
run config unset format;      check "config unset absent key" 1 "" "Key not set in persistent config: format"
run config set format;        check "config set without value" 1 "" "Usage: ukiryu config set <key> <value>"
cd "$W/author"
# This legacy fixture passes the pinned local schema. The live remote v1
# schema requires execution_profiles and Ruby rejects it there too.
run validate file defs/convert.yaml --schema schema/tool.schema.yaml; check "validate file" 0 "Valid" ""
cd "$W/exec"
run exec cat concatenate --dry-run --raw --definition defs/cat.yaml; check "exec --dry-run" 0 "DRY RUN" ""
run exec cat concatenate --format xml --definition defs/cat.yaml; check "exec rejects invalid --format" 1 "" "Invalid format: xml"
cd "$W"
run cache info;         [ "$CODE" = 0 ] && ok "cache info" || bad "cache info"

# --- verbs gated on main: PENDING until their PR merges, then must pass
verb "config list"       "PR #73" config list
verb "list"              "PR #73" list
verb "commands jq"      "PR #73" commands jq
verb "register"          "PR #73" register
# resolve reads the user/bundled/system definition directories, not the
# register, so install the register's jq definition as a user definition.
mkdir -p "$HOME/.local/share/ukiryu/definitions/jq"
cp /register/tools/jq/default/*.yaml "$HOME/.local/share/ukiryu/definitions/jq/"
verb "resolve jq"       "PR #73" resolve jq
verb "info jq"          "PR #77" info jq
verb "opts jq"          "PR #77" opts jq
verb "describe jq"      "PR #74" describe jq --register /register
verb "which jq"         "no open PR (K22-CLI-INSPECT remainder)" which jq

echo
echo "summary: $pass passed, $fail failed, $pending pending"
[ "$fail" = 0 ]
