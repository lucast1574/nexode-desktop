#!/bin/bash
# Replay a nexode recording as video
#
# This script:
# 1. Starts a fresh nexode instance in a new terminal
# 2. Records the screen with wf-recorder
# 3. Replays the recorded keystrokes with proper timing
# 4. Outputs a video file
#
# Usage: ./replay_recording.sh <recording.json> [output.mp4]

set -e

RECORDING_FILE="${1:?Usage: $0 <recording.json> [output.mp4]}"
OUTPUT_FILE="${2:-${RECORDING_FILE%.json}.mp4}"

SCRIPT_DIR="$(dirname "$0")"

if [ ! -f "$RECORDING_FILE" ]; then
    echo "Error: Recording file not found: $RECORDING_FILE"
    exit 1
fi

echo "🎬 nexode Recording Replay"
echo "   Input:  $RECORDING_FILE"
echo "   Output: $OUTPUT_FILE"
echo ""


# Get the screen geometry for recording
GEOMETRY=$(niri msg focused-output 2>/dev/null | grep -oP 'Mode: \K\d+x\d+' | head -1 || echo "1920x1080")

echo "📹 Starting screen recording..."
echo "   Geometry: $GEOMETRY"
echo ""

# Start wf-recorder in background
wf-recorder -g "0,0 $GEOMETRY" -f "$OUTPUT_FILE" &
RECORDER_PID=$!
sleep 1  # Let recorder initialize

# Start nexode in a new kitty window
echo "🚀 Starting nexode..."
kitty --title "nexode-replay" -e bash -c "cd $(pwd) && ~/.cargo/bin/nexode; read -p 'Press Enter to close...'" &
KITTY_PID=$!
sleep 2  # Wait for nexode to start

# Focus the new window
sleep 0.5
# Find and focus the nexode-replay window
WINDOW_ID=$(niri msg windows 2>/dev/null | grep -B5 "nexode-replay" | grep -oP 'Window ID \K\d+' | head -1)
if [ -n "$WINDOW_ID" ]; then
    echo "   Focusing window $WINDOW_ID"
    niri msg action focus-window --id "$WINDOW_ID"
    sleep 0.3
fi

echo "⌨️  Replaying keystrokes..."
echo ""

# Replay through native argument vectors without evaluating generated shell code.
"$SCRIPT_DIR/orbit_dev.sh" replay-keys "$RECORDING_FILE"

echo ""
echo "⏹️  Stopping recording..."
sleep 1  # Final pause

# Stop recorder
kill $RECORDER_PID 2>/dev/null || true
wait $RECORDER_PID 2>/dev/null || true

# Clean up kitty window
kill $KITTY_PID 2>/dev/null || true

echo ""
echo "✅ Done!"
echo "   Video saved to: $OUTPUT_FILE"
ls -lh "$OUTPUT_FILE"
