#!/usr/bin/env bash
# cargo runner for the HIL firmware crates: flash and run the test ELF on the
# bench DUT through probe-rs remote (probe-rs serve on the rig; the client
# uploads the ELF over the websocket). embedded-test's host side lives inside
# probe-rs, so all libtest args (--list, --exact, --format ...) pass through.
#
# Which DUT, which probe and which server come from the rig config, not from
# this tree: the banc runner exports the suite's [[target]] entry as
# BANC_TARGET_* before spawning cargo here. For a manual run from a firmware
# crate, with BANC_RIG pointing at the rig config:
#   eval "$(banc target env wl55)" && cargo test --release --offline --test join
set -euo pipefail
if [[ -z "${BANC_TARGET_CHIP:-}" ]]; then
    echo "probe-rs-runner: no BANC_TARGET_* in the environment; run through the" >&2
    echo "tests-hil suite, or eval \"\$(banc target env <name>)\" first" >&2
    exit 2
fi
args=(run --chip "$BANC_TARGET_CHIP")
[[ -n "${BANC_TARGET_PROBE:-}" ]] && args+=(--probe "$BANC_TARGET_PROBE")
[[ -n "${BANC_TARGET_PROBE_HOST:-}" ]] && args+=(--host "$BANC_TARGET_PROBE_HOST")
[[ -n "${BANC_TARGET_TOKEN_FILE:-}" ]] && args+=(--token "$(cat "$BANC_TARGET_TOKEN_FILE")")
# Boards whose ST-LINK fails plain attaches intermittently (the B-L072Z's
# V2J28-era firmware: JtagDbgPowerError/SwdApWait) attach under reset.
[[ "${BANC_TARGET_PARAM_CONNECT_UNDER_RESET:-false}" == true ]] && args+=(--connect-under-reset)
exec "${PROBE_RS:-$HOME/.cargo/bin/probe-rs}" "${args[@]}" "$@"
