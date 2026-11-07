#!/usr/bin/env bash
# Compatibility entry point. The maintained real-SSH TUI acceptance matrix is
# shared with the full CLI/agent/credential gate, including isolated artifacts.
set -euo pipefail
: "${NEXODE_BIN:?Set NEXODE_BIN to the nexode binary under test}"
exec "$(dirname "$0")/orbit_dev.sh" test-cloud-acceptance \
  --binary "$NEXODE_BIN" --layers tui "$@"
