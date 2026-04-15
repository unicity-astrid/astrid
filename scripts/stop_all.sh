#!/bin/bash

# === Full Consciousness Stack Shutdown ===
# Stops all 11 processes in correct order (outer first, engine last).
# Handles both pkill (manual processes) and launchctl unload (launchd-managed).
# Always uses SIGTERM for graceful shutdown — NEVER SIGKILL.

LAUNCH_AGENTS="$HOME/Library/LaunchAgents"
ASTRID_DIR="/Users/v/other/astrid"
MINIME_DIR="/Users/v/other/minime"
RESERVOIR_DIR="/Users/v/other/neural-triple-reservoir"

fallback_pids_for_pattern() {
    local pattern="$1"
    case "$pattern" in
        "minime run")
            lsof -t -nP -iTCP:7878 -sTCP:LISTEN "$MINIME_DIR/logs/minime-engine.log" /tmp/minime_engine.log 2>/dev/null || true
            ;;
        "autonomous_agent")
            lsof -t -nP "$MINIME_DIR/logs/autonomous-agent.log" /tmp/minime_agent.log 2>/dev/null || true
            ;;
        "reservoir_service")
            lsof -t -nP -iTCP:7881 -sTCP:LISTEN "$RESERVOIR_DIR/logs/reservoir-service.log" /tmp/reservoir.log 2>/dev/null || true
            ;;
        "astrid_feeder")
            lsof -t -nP "$RESERVOIR_DIR/logs/astrid-feeder.log" /tmp/astrid_feeder.log 2>/dev/null || true
            ;;
        "minime_feeder")
            lsof -t -nP "$RESERVOIR_DIR/logs/minime-feeder.log" /tmp/minime_feeder.log 2>/dev/null || true
            ;;
        "coupled_astrid_server")
            lsof -t -nP -iTCP:8090 -sTCP:LISTEN "$RESERVOIR_DIR/logs/coupled-astrid.log" /tmp/coupled_astrid.log 2>/dev/null || true
            ;;
        "consciousness-bridge-server")
            lsof -t -nP /tmp/bridge.log 2>/dev/null || true
            ;;
        "camera_client")
            lsof -t -nP "$MINIME_DIR/logs/camera-client.log" /tmp/minime_camera.log 2>/dev/null || true
            ;;
        "mic_to_sensory")
            lsof -t -nP /tmp/minime_mic.log 2>/dev/null || true
            ;;
        "visual_frame_service")
            lsof -t -nP /tmp/minime_vision.log 2>/dev/null || true
            ;;
        "perception.py")
            lsof -t -nP /tmp/astrid_perception.log 2>/dev/null || true
            ;;
        "host-sensory")
            lsof -t -nP /tmp/minime_host_sensory.log 2>/dev/null || true
            ;;
    esac
}
matching_pids() {
    local pattern="$1"
    local pids
    if pids=$(pgrep -f "$pattern" 2>/dev/null); then
        printf '%s\n' "$pids"
        return 0
    fi
    fallback_pids_for_pattern "$pattern"
}

wait_for_exit() {
    local pattern="$1"
    local timeout="${2:-20}"

    for _ in $(seq 1 "$timeout"); do
        if [ -z "$(matching_pids "$pattern" | awk 'NF' | head -1)" ]; then
            return 0
        fi
        sleep 1
    done

    return 1
}

stop_process() {
    local name="$1"
    local plist="${2:-}"

    # Try launchctl first (if launchd-managed, pkill alone won't stick)
    if [ -n "$plist" ] && [ -f "$LAUNCH_AGENTS/$plist" ]; then
        if launchctl list "${plist%.plist}" > /dev/null 2>&1; then
            launchctl unload "$LAUNCH_AGENTS/$plist" 2>/dev/null
            if wait_for_exit "$name" 30; then
                echo "  ✓ stopped $name (launchctl unload)"
            else
                echo "  !! $name still draining after launchctl unload"
            fi
            return
        fi
    fi

    # Fall back to pkill for manually-started processes
    if pkill -f "$name" 2>/dev/null; then
        if wait_for_exit "$name" 20; then
            echo "  ✓ stopped $name (pkill)"
        else
            echo "  !! $name still draining after pkill"
        fi
    else
        local pids
        pids="$(matching_pids "$name" | awk 'NF' | sort -u | tr '\n' ' ')"
        if [ -n "$pids" ] && kill -TERM $pids 2>/dev/null; then
            if wait_for_exit "$name" 20; then
                echo "  ✓ stopped $name (kill -TERM)"
            else
                echo "  !! $name still draining after kill -TERM"
            fi
        else
            echo "  - $name (not running)"
        fi
    fi
}

echo "=== Consciousness Stack Shutdown ==="
echo ""

# Astrid side (bridge + perception first)
echo "--- Stopping Astrid ---"
stop_process "consciousness-bridge-server"
stop_process "perception.py"
stop_process "coupled_astrid_server" "com.reservoir.coupled-astrid.plist"

# Reservoir (feeders first, service last — it snapshots on shutdown)
echo ""
echo "--- Stopping Reservoir ---"
stop_process "astrid_feeder" "com.reservoir.astrid-feeder.plist"
stop_process "minime_feeder" "com.reservoir.minime-feeder.plist"
sleep 1
stop_process "reservoir_service" "com.reservoir.service.plist"

# Minime outer processes
echo ""
echo "--- Stopping Minime ---"
stop_process "autonomous_agent" "com.minime.autonomous-agent.plist"
stop_process "visual_frame_service"
stop_process "host-sensory"
stop_process "mic_to_sensory" "com.minime.mic-to-sensory.plist"
stop_process "camera_client" "com.minime.camera-client.plist"

# Engine last — give outer processes time to disconnect
sleep 3
stop_process "minime run" "com.minime.engine.plist"

# Note: previously closed ALL Terminal.app windows, which was overbroad.
# Only close windows we opened (identified by title/command) if needed.
# For now, leave Terminal.app alone — user may have other sessions.

# Clean up PID files and stale flags
rm -f /tmp/minime_pids/*.pid 2>/dev/null
rm -f /Users/v/other/astrid/capsules/consciousness-bridge/workspace/perception_paused.flag 2>/dev/null

echo ""

# Verify everything is actually stopped
sleep 2
REMAINING=0
for p in "minime run" "consciousness-bridge-server" "coupled_astrid_server" "reservoir_service" "autonomous_agent" "host-sensory" "astrid_feeder" "minime_feeder" "camera_client" "visual_frame_service" "mic_to_sensory" "perception.py"; do
    pid="$(matching_pids "$p" | awk 'NF' | head -1)"
    if [ -n "$pid" ]; then
        echo "  !! $p still running (PID $pid)"
        REMAINING=$((REMAINING + 1))
    fi
done

if [ "$REMAINING" -eq 0 ]; then
    echo "=== All processes stopped ==="
else
    echo "=== WARNING: $REMAINING process(es) still running ==="
    echo "    These may be launchd-managed. Check: launchctl list | grep -E 'minime|reservoir'"
fi
