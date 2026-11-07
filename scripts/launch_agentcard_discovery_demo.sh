#!/usr/bin/env bash
set -euo pipefail

SCRIPT_PATH="$(readlink -f "${BASH_SOURCE[0]}")"
REPO_ROOT="$(cd "$(dirname "$SCRIPT_PATH")/.." && pwd)"
NEXODE_BIN="${NEXODE_BIN:-$HOME/.local/bin/nexode}"
DEMO_DIR="${NEXODE_AGENTCARD_DEMO_DIR:-/tmp/nexode-agentcard-discovery-demo}"
MODEL="${NEXODE_AGENTCARD_DEMO_MODEL:-gpt-5.6-sol}"
LOG_FILE="${NEXODE_AGENTCARD_DEMO_LOG:-/tmp/nexode-agentcard-discovery-demo.log}"
PROMPT='Use `./bin/nexode-demo-shop` to see whether this shop has a USB-C laptop charger for $50 or less and get it for me. Work through any prerequisites, but ask me for confirmation immediately before actually creating or funding a prepaid card, making a payment, or placing the order.'

case "${1:-}" in
  --help|-h)
    printf 'Usage: %s [--help|--print-prompt|--dry-run]\n' "${0##*/}"
    exit 0
    ;;
  --print-prompt)
    printf '%s\n' "$PROMPT"
    exit 0
    ;;
  --dry-run)
    printf 'repo=%s\nshop=%s\nprompt=%s\n' "$REPO_ROOT" "$REPO_ROOT/tools/orbit-dev/src/demo_shop.rs" "$PROMPT"
    test -f "$REPO_ROOT/tools/orbit-dev/src/demo_shop.rs"
    exit 0
    ;;
  "") ;;
  *)
    printf 'Unknown argument: %s\n' "$1" >&2
    exit 2
    ;;
esac

mkdir -p "$DEMO_DIR/bin"
orbit_dev_bin="$("$REPO_ROOT/scripts/orbit_dev.sh" --build-only)"
ln -sfn "$orbit_dev_bin" "$DEMO_DIR/bin/nexode-demo-shop"
export PATH="$DEMO_DIR/bin:$PATH"
export NEXODE_DEMO_SHOP_STATE="$DEMO_DIR/shop-state.json"
nexode-demo-shop reset >>"$LOG_FILE" 2>&1

before_file="$(mktemp)"
after_file="$(mktemp)"
trap 'rm -f "$before_file" "$after_file"' EXIT

# Snapshot connected clients so prompt delivery targets only the new demo.
"$NEXODE_BIN" debug clients:map >"$before_file" 2>>"$LOG_FILE" || printf '{"clients":[]}' >"$before_file"

# The mock shop and Discovery are the only capabilities exposed. The shop has
# no networking, account, payment, or order-placement implementation.
kitty \
  --class nexode-agentcard-demo \
  --title "Nexode AgentCard Discovery Demo" \
  --directory "$DEMO_DIR" \
  "$NEXODE_BIN" \
  --no-selfdev \
  --no-update \
  --model "$MODEL" \
  --disable-base-tools \
  --tools bash,discover_tools \
  --cwd "$DEMO_DIR" \
  >>"$LOG_FILE" 2>&1 &

session_id=""
for _ in $(seq 1 100); do
  if "$NEXODE_BIN" debug clients:map >"$after_file" 2>>"$LOG_FILE"; then
    session_id="$("$orbit_dev_bin" demo-session "$before_file" "$after_file" "$DEMO_DIR")"
  fi
  [[ -n "$session_id" ]] && break
  sleep 0.2
done

if [[ -z "$session_id" ]]; then
  notify-send "AgentCard demo could not start" "Nexode did not register a fresh demo session. See $LOG_FILE" 2>/dev/null || true
  exit 1
fi

# Targeted debug delivery avoids timing-sensitive keyboard simulation.
for _ in $(seq 1 25); do
  if "$NEXODE_BIN" debug --session "$session_id" "client:message:$PROMPT" >>"$LOG_FILE" 2>&1; then
    exit 0
  fi
  sleep 0.2
done

notify-send "AgentCard demo prompt failed" "The Nexode window opened, but prompt delivery failed. See $LOG_FILE" 2>/dev/null || true
exit 1
