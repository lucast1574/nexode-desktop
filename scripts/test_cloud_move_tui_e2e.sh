#!/usr/bin/env bash
# Compatibility entry point. The maintained real-SSH TUI acceptance matrix is
# shared with the full CLI/agent/credential gate, including isolated artifacts.
set -euo pipefail
: "${NEXODE_BIN:?Set NEXODE_BIN to the nexode binary under test}"
exec python3 "$(dirname "$0")/test_cloud_acceptance.py" \
  --binary "$NEXODE_BIN" --layers tui "$@"
