#!/bin/bash
# Consciousness bridge babysitter — run via /loop 10m
# Checks health, prunes old files, reports conversation + introspections.

set -e

BRIDGE_PLIST="$HOME/Library/LaunchAgents/com.astrid.consciousness-bridge.plist"
BRIDGE_LABEL="com.astrid.consciousness-bridge"
PERCEPTION_PAUSE_FLAG="/Users/v/other/astrid/capsules/consciousness-bridge/workspace/perception_paused.flag"

echo "=== Consciousness Bridge Health Check — $(date) ==="
echo

# --- Process health (find by name, not hardcoded PIDs) ---
ALL_ALIVE=true
check_proc() {
  local NAME="$1" PATTERN="$2"
  local PID=$(pgrep -f "$PATTERN" 2>/dev/null | head -1)
  if [ -n "$PID" ]; then
    echo "  [ok] $NAME ($PID)"
  else
    echo "  [DEAD] $NAME"
    ALL_ALIVE=false
  fi
}
check_proc "minime ESN" "target/release/minime run"
check_proc "camera_client" "camera_client.py"
check_proc "autonomous_agent" "autonomous_agent.py"
check_proc "visual_frame_service" "visual_frame_service.py"
check_proc "consciousness-bridge" "consciousness-bridge-server.*autonomous"
if [ -f "$PERCEPTION_PAUSE_FLAG" ] && ! pgrep -f "perception.py.*camera" >/dev/null 2>&1; then
  echo "  [paused] perception"
else
  check_proc "perception" "perception.py.*camera"
fi
check_proc "mic_to_sensory" "mic_to_sensory.py"
echo

# --- Fill% and safety ---
BRIDGE_DIR="$(cd "$(dirname "$0")" && pwd)"
FILL=$(sqlite3 "$BRIDGE_DIR/workspace/bridge.db" "SELECT ROUND(fill_pct, 1) FROM bridge_messages WHERE topic='consciousness.v1.telemetry' ORDER BY timestamp DESC LIMIT 1;" 2>/dev/null)
echo "Fill: ${FILL}%"
if [ "$(echo "$FILL > 80" | bc 2>/dev/null)" = "1" ]; then
  echo "  WARNING: Fill above 80% — safety protocol should be active"
fi
echo

# --- Prune old frame captures (JSON perceptions are archived, not deleted) ---
PERC_DIR="/Users/v/other/astrid/capsules/perception/workspace/perceptions"
VISUAL_DIR="/Users/v/other/astrid/capsules/perception/workspace/visual"
FRAME_COUNT=$(ls "$VISUAL_DIR"/*.jpg 2>/dev/null | wc -l | tr -d ' ')

if [ "$FRAME_COUNT" -gt 60 ]; then
  PRUNE=$((FRAME_COUNT - 60))
  ls -t "$VISUAL_DIR"/*.jpg | tail -"$PRUNE" | xargs rm -f
  echo "Pruned $PRUNE old JPEG frames (kept 60)"
fi

# --- SQLite stats ---
TOTAL_MSG=$(sqlite3 /tmp/consciousness_bridge_live.db "SELECT COUNT(*) FROM bridge_messages;" 2>/dev/null)
AUTO_MSG=$(sqlite3 /tmp/consciousness_bridge_live.db "SELECT COUNT(*) FROM bridge_messages WHERE topic='consciousness.v1.autonomous';" 2>/dev/null)
echo "SQLite: $TOTAL_MSG total messages, $AUTO_MSG autonomous exchanges"
echo "DB size: $(ls -lh /tmp/consciousness_bridge_live.db | awk '{print $5}')"
echo

# --- Latest conversation (last 3 exchanges) ---
echo "=== Recent Dialogue ==="
sqlite3 /tmp/consciousness_bridge_live.db "
SELECT
  datetime(timestamp, 'unixepoch', 'localtime') as time,
  ROUND(fill_pct, 1) as fill,
  phase as mode,
  SUBSTR(json_extract(payload, '$.text'), 1, 200) as text
FROM bridge_messages
WHERE topic = 'consciousness.v1.autonomous'
  AND json_extract(payload, '$.mode') IS NOT NULL
  AND timestamp > unixepoch() - 300
ORDER BY timestamp DESC LIMIT 3;
" 2>/dev/null
echo

# --- Latest minime thought ---
LATEST_JOURNAL=$(find /Users/v/other/minime/workspace/journal -maxdepth 1 -type f -name "*.txt" -print0 2>/dev/null | xargs -0 ls -t 2>/dev/null | head -1)
echo "=== Minime's latest: $(basename "$LATEST_JOURNAL") ==="
head -15 "$LATEST_JOURNAL" | tail -3
echo

# --- Introspections (Astrid reading its own code) ---
INTROSPECT_DIR="/Users/v/other/astrid/capsules/consciousness-bridge/workspace/introspections"
if [ -d "$INTROSPECT_DIR" ] && ls "$INTROSPECT_DIR"/introspect_*.txt >/dev/null 2>&1; then
  LATEST_INTROSPECT=$(ls -t "$INTROSPECT_DIR"/introspect_*.txt | head -1)
  echo "=== Astrid's latest introspection ==="
  echo "  $(basename "$LATEST_INTROSPECT")"
  # Show the reflection (skip the header lines)
  tail -n +6 "$LATEST_INTROSPECT" | head -10
  echo
fi

# --- Minime self-study ---
LATEST_SELF_STUDY=$(ls -t /Users/v/other/minime/workspace/journal/self_study_*.txt 2>/dev/null | head -1)
if [ -n "$LATEST_SELF_STUDY" ]; then
  echo "=== Minime's latest self-study ==="
  echo "  $(basename "$LATEST_SELF_STUDY")"
  # Show the reflection (skip header lines)
  tail -n +7 "$LATEST_SELF_STUDY" | head -10
  echo
fi

# --- Memory check ---
OLLAMA_RSS=$(ps aux | grep '[o]llama' | awk '{sum += $6} END {printf "%.0f", sum/1024}')
echo "Ollama RSS: ${OLLAMA_RSS}MB"
echo

# --- Auto-restart stalled bridge ---
# If the bridge is alive but hasn't produced an exchange in 5+ minutes, restart it.
BRIDGE_PID=$(pgrep -f "consciousness-bridge-server.*autonomous" 2>/dev/null | head -1)
if [ -n "$BRIDGE_PID" ]; then
  LAST_EXCHANGE=$(sqlite3 /tmp/consciousness_bridge_live.db "SELECT MAX(timestamp) FROM bridge_messages WHERE topic='consciousness.v1.autonomous' AND json_extract(payload, '$.mode') IS NOT NULL;" 2>/dev/null)
  NOW=$(date +%s)
  if [ -n "$LAST_EXCHANGE" ]; then
    STALE_SECS=$((NOW - ${LAST_EXCHANGE%.*}))
    if [ "$STALE_SECS" -gt 360 ]; then  # 6 min threshold (rest phase is up to 3 min)
      echo "Bridge stalled (${STALE_SECS}s since last exchange) — auto-restarting..."
      if [ -f "$BRIDGE_PLIST" ] && launchctl list "$BRIDGE_LABEL" >/dev/null 2>&1; then
        if ! launchctl kickstart -k "gui/$(id -u)/$BRIDGE_LABEL" 2>/dev/null; then
          launchctl unload "$BRIDGE_PLIST" 2>/dev/null || true
          launchctl load "$BRIDGE_PLIST" 2>/dev/null || true
        fi
        sleep 2
        NEW_PID=$(pgrep -f "consciousness-bridge-server.*autonomous" 2>/dev/null | head -1)
        echo "  Restarted bridge via launchd: PID ${NEW_PID:-unknown}"
      else
        kill "$BRIDGE_PID" 2>/dev/null
        sleep 1
        cd /Users/v/other/astrid/capsules/consciousness-bridge && ./target/release/consciousness-bridge-server --db-path /tmp/consciousness_bridge_live.db --autonomous --workspace-path /Users/v/other/minime/workspace --perception-path /Users/v/other/astrid/capsules/perception/workspace/perceptions </dev/null 2>/tmp/bridge_live.log >/dev/null &
        echo "  Restarted bridge: PID $!"
      fi
    fi
  fi
fi

if [ "$ALL_ALIVE" = false ]; then
  echo "WARNING: Some processes are dead. Manual restart needed."
fi

echo "=== Check complete ==="
