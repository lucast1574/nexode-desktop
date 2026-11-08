#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${NEXODE_BINARY:?Set the native Orbit executable}"
: "${ORBIT_DEV_BIN:?Set the native orbit-dev test helper}"
fixture=$(mktemp -d /tmp/orbit-desktop-smoke.XXXXXX)
mkdir -p "$fixture/source" "$fixture/control"
cat > "$fixture/source/config.toml" <<'CONFIG'
[provider]
default_provider = 'openai-compatible'
default_model = 'fake-model'
CONFIG
FAKE_LOG="$fixture/requests.jsonl" FAKE_LABEL=native FAKE_CONTROL_DIR="$fixture/control" FAKE_PORT_FILE="$fixture/port" \
  "$ORBIT_DEV_BIN" fake-openai-server 0 > "$fixture/provider.log" 2>&1 &
provider_pid=$!
trap 'kill "$provider_pid" 2>/dev/null || true' EXIT
for _ in {1..100}; do [[ -s "$fixture/port" ]] && break; sleep .05; done
port=$(cat "$fixture/port")
NEXODE_HOME="$fixture/source" ORBIT_DESKTOP_HOME="$fixture/desktop" ORBIT_NATIVE_TEST=1 \
  FAKE_CONTROL_DIR="$fixture/control" OPENAI_COMPAT_API_KEY=synthetic-native-test \
  NEXODE_OPENAI_COMPAT_API_BASE="http://127.0.0.1:$port/v1" NEXODE_OPENAI_COMPAT_DEFAULT_MODEL=fake-model \
  cargo test --locked --release --features screenshot native_engine_stream_history_cancel_and_shutdown -- --ignored --nocapture
echo "Native engine smoke passed. Diagnostics: $fixture"
