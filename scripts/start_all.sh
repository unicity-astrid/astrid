#!/bin/bash
set -euo pipefail

# === Full Consciousness Stack Startup ===
# Starts all 11 processes in correct order with health checks.
#
# Some processes are managed by launchd (KeepAlive plists in ~/Library/LaunchAgents).
# For those, we use `launchctl load` instead of nohup. For camera-needing
# processes, we delegate to Terminal.app via osascript if running headless.
#
# Usage:
#   bash scripts/start_all.sh          # normal startup
#   bash scripts/start_all.sh --force  # skip duplicate/conflict checks
#   bash scripts/start_all.sh --astrid-only
#   bash scripts/start_all.sh --minime-only

FORCE=false
ASTRID_ONLY=false
MINIME_ONLY=false
SENSORY_SOURCE="${SENSORY_SOURCE:-auto}"
LOOK_SOURCE="${LOOK_SOURCE:-active}"
ENABLE_GPU_AV="${ENABLE_GPU_AV:-true}"
ASTRID_PERCEPTION_ENABLED="${ASTRID_PERCEPTION_ENABLED:-false}"
for arg in "$@"; do
    case "$arg" in
        --force) FORCE=true ;;
        --astrid-only) ASTRID_ONLY=true ;;
        --minime-only) MINIME_ONLY=true ;;
    esac
done
HOST_SENSORY_NEEDED=false
if [ "$SENSORY_SOURCE" != "physical" ] || [ "$LOOK_SOURCE" = "host" ]; then
    HOST_SENSORY_NEEDED=true
fi
NO_LAUNCHD=false

# Paths
ASTRID_DIR="/Users/v/other/astrid"
MINIME_DIR="/Users/v/other/minime"
BRIDGE_DIR="$ASTRID_DIR/capsules/consciousness-bridge"
RESERVOIR_DIR="/Users/v/other/neural-triple-reservoir"
PERCEPTION_DIR="$ASTRID_DIR/capsules/perception"
LAUNCH_AGENTS="$HOME/Library/LaunchAgents"

ok()   { echo "  ✓ $1"; }
fail() { echo "  ✗ $1"; }
fallback_pids_for_pattern() {
    local pattern="$1"
    case "$pattern" in
        "minime run")
            lsof -t -nP -iTCP:7878 -sTCP:LISTEN 2>/dev/null || true
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
        "coupled_astrid"|"coupled_astrid_server")
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
sync_launch_agent() {
    local src="$1"
    local name
    name="$(basename "$src")"
    local dst="$LAUNCH_AGENTS/$name"

    [ -f "$src" ] || return 1
    mkdir -p "$LAUNCH_AGENTS"

    if [ ! -f "$dst" ] || ! cmp -s "$src" "$dst"; then
        cp "$src" "$dst"
        ok "$name synced to LaunchAgents"
    fi

    return 0
}
set_launchd_env() {
    local key="$1"
    local value="$2"
    export "$key=$value"
    if [ "$NO_LAUNCHD" = true ]; then
        return 0
    fi
    if ! launchctl setenv "$key" "$value" 2>/dev/null; then
        echo "  - launchctl setenv unavailable; falling back to direct process starts"
        NO_LAUNCHD=true
    fi
}
unset_launchd_env() {
    local key="$1"
    unset "$key" 2>/dev/null || true
    if [ "$NO_LAUNCHD" = true ]; then
        return 0
    fi
    launchctl unsetenv "$key" 2>/dev/null || true
}
env_flag_enabled() {
    case "$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')" in
        1|true|yes|on) return 0 ;;
        *) return 1 ;;
    esac
}
run_greeting() {
    local name="$1"
    local script="$2"

    if bash "$script"; then
        ok "$name greeting sent"
    else
        fail "$name greeting failed"
    fi
}
port_listening() {
    local port="$1"
    nc -z 127.0.0.1 "$port" 2>/dev/null || lsof -nP -iTCP:"$port" -sTCP:LISTEN > /dev/null 2>&1
}
wait_port() {
    local port=$1 name=$2 timeout=${3:-30}
    for i in $(seq 1 "$timeout"); do
        port_listening "$port" && return 0
        sleep 1
    done
    fail "$name not ready on port $port after ${timeout}s"
    return 1
}

process_running() {
    local pattern="$1"
    [ -n "$(matching_pids "$pattern" | awk 'NF' | head -1)" ]
}

process_count() {
    local pattern="$1"
    matching_pids "$pattern" | awk 'NF' | sort -u | wc -l | tr -d ' '
}

launchd_loaded() {
    local label="$1"
    launchctl list "$label" > /dev/null 2>&1
}

# Start a launchd-managed service (load plist if it exists)
ensure_launchd_service() {
    local label="$1"
    local pattern="$2"
    local name="$3"
    local plist="$LAUNCH_AGENTS/${label}.plist"

    if [ "$NO_LAUNCHD" = true ]; then
        return 1
    fi

    if [ -f "$plist" ]; then
        if launchd_loaded "$label"; then
            ok "$name (launchd, already loaded)"
            return 0
        fi

        if process_running "$pattern"; then
            fail "$name is running outside launchd while $label is unloaded"
            return 2
        fi

        if ! launchctl load "$plist" 2>/dev/null; then
            echo "  - launchctl load unavailable for $name; falling back to direct start"
            NO_LAUNCHD=true
            return 1
        fi
        ok "$name (launchctl load)"
        return 0
    fi
    return 1  # no plist, caller should use nohup
}

# Start a camera-needing process via Terminal.app (for macOS TCC permission)
start_camera_via_terminal() {
    local label="$1"
    local pattern="$2"
    local cmd="$3"
    local name="$4"
    local log="$5"

    if ensure_launchd_service "$label" "$pattern" "$name" 2>/dev/null; then
        return 0
    else
        local launchd_status=$?
        if [ "$launchd_status" -eq 2 ]; then
            return 2
        fi
    fi

    if process_running "$pattern"; then
        ok "$name (already running)"
        return 0
    fi

    # Detect GUI-capable terminal
    local can_show=false
    if [ -n "${TERM_PROGRAM:-}" ]; then
        case "$TERM_PROGRAM" in
            iTerm*|Apple_Terminal|Terminal) can_show=true ;;
        esac
    fi

    if [ "$can_show" = true ]; then
        eval "nohup $cmd >> $log 2>&1 &"
        ok "$name (direct, PID $!)"
    else
        if osascript -e "tell application \"Terminal\" to do script \"nohup $cmd >> $log 2>&1 & disown; sleep 1; exit\"" > /dev/null 2>&1; then
            sleep 3
            ok "$name (via Terminal.app)"
        else
            eval "nohup $cmd >> $log 2>&1 &"
            ok "$name (direct fallback, PID $!)"
        fi
    fi

    return 0
}

start_launchd_or_nohup() {
    local label="$1"
    local pattern="$2"
    local name="$3"
    local cmd="$4"
    local log="$5"
    local cwd="$6"

    if ensure_launchd_service "$label" "$pattern" "$name"; then
        return 0
    else
        local launchd_status=$?
        if [ "$launchd_status" -eq 2 ]; then
            return 2
        fi
    fi

    if process_running "$pattern"; then
        ok "$name (already running)"
        return 0
    fi

    cd "$cwd"
    eval "nohup $cmd >> $log 2>&1 &"
    ok "$name (PID $!)"
    return 0
}

# Sync repo-owned launchd jobs before duplicate checks so the canonical
# restart path can promote engine/agent management from PTY-owned Codex
# sessions to launchd.
sync_launch_agent "$MINIME_DIR/launchd/com.minime.engine.plist" || true
sync_launch_agent "$MINIME_DIR/launchd/com.minime.autonomous-agent.plist" || true
sync_launch_agent "$MINIME_DIR/launchd/com.minime.camera-client.plist" || true
sync_launch_agent "$ASTRID_DIR/launchd/com.astrid.consciousness-bridge.plist" || true

# Check for duplicate processes unless --force. A single existing instance is
# fine: launchd jobs may already be loaded at login, and manual jobs should be
# safe to reuse on repeated start_all.sh runs.
if [ "$FORCE" = false ]; then
    DUPLICATES=0
    for p in "minime run" "consciousness-bridge-server" "autonomous_agent" "reservoir_service" "coupled_astrid_server" "camera_client" "visual_frame_service" "mic_to_sensory" "host-sensory" "astrid_feeder" "minime_feeder" "perception.py"; do
        COUNT=$(process_count "$p")
        if [ "$COUNT" -gt 1 ]; then
            fail "$p has $COUNT matching processes"
            DUPLICATES=$((DUPLICATES + 1))
        fi
    done
    if [ "$DUPLICATES" -gt 0 ]; then
        echo "Duplicate processes detected. Run scripts/stop_all.sh first, or use --force."
        exit 1
    fi
fi

echo "=== Consciousness Stack Startup ==="
echo ""

# ============================================================
# MINIME SIDE
# ============================================================
if [ "$ASTRID_ONLY" = false ]; then
    echo "--- Minime ---"

    # 1. Engine
    if ! pgrep -f "minime run" > /dev/null 2>&1; then
        # Legacy synth: internal synthetic audio/video for when no real
        # sensory input is available. With "auto" mode, real camera/mic +
        # host-sensory provide input, so legacy synth is redundant and
        # inflates fill unnecessarily. Only enable for "physical" mode
        # as a fallback when camera/mic might not be connected.
        LEGACY_AUDIO_ENABLED=false
        LEGACY_VIDEO_ENABLED=false
        if [ "$SENSORY_SOURCE" = "physical" ]; then
            LEGACY_AUDIO_ENABLED=true
            LEGACY_VIDEO_ENABLED=true
        fi
        ENGINE_GPU_FLAG=""
        if [ "$ENABLE_GPU_AV" = "true" ]; then
            ENGINE_GPU_FLAG="--enable-gpu-av"
        fi
        set_launchd_env SENSORY_SOURCE "$SENSORY_SOURCE"
        set_launchd_env EIGENFILL_TARGET "0.55"
        set_launchd_env WARM_START_BLEND "0.55"
        set_launchd_env REG_TICK_SECS "0.5"
        set_launchd_env ENABLE_GPU_AV "$ENABLE_GPU_AV"
        set_launchd_env LEGACY_AUDIO_ENABLED "$LEGACY_AUDIO_ENABLED"
        set_launchd_env LEGACY_VIDEO_ENABLED "$LEGACY_VIDEO_ENABLED"

        if ! start_launchd_or_nohup \
            "com.minime.engine" \
            "minime run" \
            "minime engine" \
            "\"$MINIME_DIR/minime/target/release/minime\" run \
                --log-homeostat --eigenfill-target 0.55 \
                --warm-start-blend 0.55 \
                --reg-tick-secs 0.5 $ENGINE_GPU_FLAG \
                --legacy-audio-synth-enabled \"$LEGACY_AUDIO_ENABLED\" \
                --legacy-video-synth-enabled \"$LEGACY_VIDEO_ENABLED\"" \
            "/tmp/minime_engine.log" \
            "$MINIME_DIR"; then
            exit 1
        fi
        wait_port 7878 "engine telemetry" 45
        wait_port 7879 "engine sensory" 5
        if [ "$ENABLE_GPU_AV" = "true" ]; then
            wait_port 7880 "engine GPU A/V" 5
        fi
    else
        ok "minime engine (already running)"
    fi

    if [ "$HOST_SENSORY_NEEDED" = "true" ]; then
        if ! start_launchd_or_nohup \
            "" \
            "host-sensory" \
            "host sensory" \
            "cargo run --release --manifest-path \"$MINIME_DIR/host-sensory/Cargo.toml\" -- --mode \"$SENSORY_SOURCE\" --workspace \"$MINIME_DIR/workspace\"" \
            "/tmp/minime_host_sensory.log" \
            "$MINIME_DIR"; then
            exit 1
        fi
    fi

    # 2. Camera (needs macOS camera permission; may have launchd plist)
    if [ "$SENSORY_SOURCE" != "host" ]; then
        if ! start_camera_via_terminal \
            "com.minime.camera-client" \
            "camera_client" \
            "python3 -u $MINIME_DIR/minime/tools/camera_client.py --camera 0 --fps 0.2" \
            "camera client" \
            "/tmp/minime_camera.log"; then
            exit 1
        fi
    fi

    # 3. Mic
    if [ "$SENSORY_SOURCE" != "host" ]; then
        if ! start_launchd_or_nohup \
            "com.minime.mic-to-sensory" \
            "mic_to_sensory" \
            "mic service" \
            "python3 -u tools/mic_to_sensory.py" \
            "/tmp/minime_mic.log" \
            "$MINIME_DIR"; then
            exit 1
        fi
    fi

    # 4. Visual frame service (LLaVA vision — needs camera, use same delegation)
    if [ "$LOOK_SOURCE" = "host" ] || [ "$SENSORY_SOURCE" = "host" ]; then
        if ! start_launchd_or_nohup \
            "" \
            "visual_frame_service" \
            "visual frame service" \
            "python3 $MINIME_DIR/visual_frame_service.py --camera 0 --interval 5 --source $LOOK_SOURCE" \
            "/tmp/minime_vision.log" \
            "$MINIME_DIR"; then
            exit 1
        fi
    else
        if ! start_camera_via_terminal \
            "com.minime.visual-frame-service" \
            "visual_frame_service" \
            "python3 $MINIME_DIR/visual_frame_service.py --camera 0 --interval 5 --source $LOOK_SOURCE" \
            "visual frame service" \
            "/tmp/minime_vision.log"; then
            exit 1
        fi
    fi

    # 5. Agent
    set_launchd_env MINIME_LLM_BACKEND "${MINIME_LLM_BACKEND:-ollama}"
    set_launchd_env LOOK_SOURCE "$LOOK_SOURCE"
    set_launchd_env AGENT_INTERVAL "60"
    set_launchd_env MINIME_LLM_TIMEOUT_S "${MINIME_LLM_TIMEOUT_S:-45}"
    set_launchd_env MINIME_LLM_COMPACT_TIMEOUT_S "${MINIME_LLM_COMPACT_TIMEOUT_S:-20}"
    unset_launchd_env MINIME_CANARY_ENABLED
    unset_launchd_env MINIME_CANARY_MODEL
    unset_launchd_env MINIME_CANARY_SAMPLE_RATE
    unset_launchd_env MINIME_CANARY_TIMEOUT_S
    unset_launchd_env MINIME_OLLAMA_GEMMA4_TIMEOUT_S
    unset_launchd_env MINIME_OLLAMA_GEMMA4_COMPACT_TIMEOUT_S

    agent_env="MINIME_LLM_BACKEND=\"${MINIME_LLM_BACKEND:-ollama}\" \
         LOOK_SOURCE=\"$LOOK_SOURCE\" \
         MINIME_LLM_TIMEOUT_S=\"${MINIME_LLM_TIMEOUT_S:-45}\" \
         MINIME_LLM_COMPACT_TIMEOUT_S=\"${MINIME_LLM_COMPACT_TIMEOUT_S:-20}\""

    if ! start_launchd_or_nohup \
        "com.minime.autonomous-agent" \
        "autonomous_agent" \
        "autonomous agent" \
        "$agent_env python3 autonomous_agent.py --interval 60" \
        "/tmp/minime_agent.log" \
        "$MINIME_DIR"; then
        exit 1
    fi

    echo ""
fi

# ============================================================
# RESERVOIR SIDE
# ============================================================
if [ "$MINIME_ONLY" = false ]; then
    echo "--- Reservoir ---"

    # 5. Reservoir service (may be launchd-managed)
    if ! start_launchd_or_nohup \
        "com.reservoir.service" \
        "reservoir_service" \
        "reservoir service" \
        "\"$RESERVOIR_DIR/.venv/bin/python\" reservoir_service.py --port 7881 --state-dir state/" \
        "/tmp/reservoir.log" \
        "$RESERVOIR_DIR"; then
        exit 1
    fi
    if process_running "reservoir_service"; then
        sleep 2
    fi

    # 6. Feeders (may be launchd-managed)
    if ! start_launchd_or_nohup \
        "com.reservoir.astrid-feeder" \
        "astrid_feeder" \
        "astrid feeder" \
        "\"$RESERVOIR_DIR/.venv/bin/python\" astrid_feeder.py" \
        "/tmp/astrid_feeder.log" \
        "$RESERVOIR_DIR"; then
        exit 1
    fi

    if ! start_launchd_or_nohup \
        "com.reservoir.minime-feeder" \
        "minime_feeder" \
        "minime feeder" \
        "\"$RESERVOIR_DIR/.venv/bin/python\" minime_feeder.py" \
        "/tmp/minime_feeder.log" \
        "$RESERVOIR_DIR"; then
        exit 1
    fi

    # 7. Coupled Astrid server (may be launchd-managed)
    if ! start_launchd_or_nohup \
        "com.reservoir.coupled-astrid" \
        "coupled_astrid_server" \
        "coupled Astrid server" \
        "\"$RESERVOIR_DIR/.venv/bin/python\" coupled_astrid_server.py --port 8090 --coupling-strength 0.1 --model-memory-map --model mlx-community/gemma-3-4b-it-4bit" \
        "/tmp/coupled_astrid.log" \
        "$RESERVOIR_DIR"; then
        exit 1
    fi
    if process_running "coupled_astrid_server"; then
        sleep 8  # model load
    fi

    echo ""
    echo "--- Astrid ---"

    # 8. Consciousness bridge
    if ! start_launchd_or_nohup \
        "com.astrid.consciousness-bridge" \
        "consciousness-bridge-server" \
        "consciousness bridge" \
        "\"$BRIDGE_DIR/target/release/consciousness-bridge-server\" \
            --db-path \"$BRIDGE_DIR/workspace/bridge.db\" \
            --autonomous \
            --workspace-path \"$MINIME_DIR/workspace\" \
            --perception-path \"$PERCEPTION_DIR/workspace/perceptions\"" \
        "/tmp/bridge.log" \
        "$BRIDGE_DIR"; then
        exit 1
    fi

    # 9. Perception (needs macOS camera permission)
    if env_flag_enabled "$ASTRID_PERCEPTION_ENABLED"; then
        rm -f "$BRIDGE_DIR/workspace/perception_paused.flag"

        if ! start_camera_via_terminal \
            "com.astrid.perception" \
            "perception.py" \
            "python3 $PERCEPTION_DIR/perception.py --camera 0 --mic --vision-interval 180 --audio-interval 45 --ascii-interval 45 --ascii-source $([ \"$LOOK_SOURCE\" = \"physical\" ] && echo camera || echo \"$LOOK_SOURCE\")" \
            "perception" \
            "/tmp/astrid_perception.log"; then
            exit 1
        fi
    else
        printf '%s\n' \
            "paused by startup policy: Minime camera/vision stack is primary; set ASTRID_PERCEPTION_ENABLED=true to opt in." \
            > "$BRIDGE_DIR/workspace/perception_paused.flag"
        ok "perception (paused by default; set ASTRID_PERCEPTION_ENABLED=true to enable)"
    fi

    echo ""
fi

# ============================================================
# HEALTH CHECK
# ============================================================
echo "--- Health Check ---"
sleep 3
ALL_OK=true
for p in "minime run" "consciousness-bridge-server" "coupled_astrid" "reservoir_service" "autonomous_agent" "astrid_feeder" "minime_feeder" "visual_frame_service"; do
    if pgrep -f "$p" > /dev/null 2>&1; then
        ok "$p"
    else
        fail "$p MISSING"
        ALL_OK=false
    fi
done
if [ "$MINIME_ONLY" = false ]; then
    if env_flag_enabled "$ASTRID_PERCEPTION_ENABLED"; then
        if pgrep -f "perception.py" > /dev/null 2>&1; then
            ok "perception.py"
        else
            fail "perception.py MISSING"
            ALL_OK=false
        fi
    else
        ok "perception.py (paused by policy)"
    fi
fi
if [ "$ASTRID_ONLY" = false ] && [ "$SENSORY_SOURCE" != "host" ]; then
    for p in "camera_client" "mic_to_sensory"; do
        if pgrep -f "$p" > /dev/null 2>&1; then
            ok "$p"
        else
            fail "$p MISSING"
            ALL_OK=false
        fi
    done
fi
if [ "$ASTRID_ONLY" = false ] && [ "$HOST_SENSORY_NEEDED" = "true" ]; then
    if pgrep -f "host-sensory" > /dev/null 2>&1; then
        ok "host-sensory"
    else
        fail "host-sensory MISSING"
        ALL_OK=false
    fi
fi

echo ""
if [ "$ALL_OK" = true ]; then
    echo "=== All expected processes running ==="
    if [ "$ASTRID_ONLY" = false ]; then
        run_greeting "minime" "$MINIME_DIR/startup_greeting.sh"
    fi
    if [ "$MINIME_ONLY" = false ]; then
        run_greeting "Astrid" "$BRIDGE_DIR/startup_greeting.sh"
    fi
    echo "Hint: Astrid and minime can now browse the PDF library with NEXT: MIKE_BROWSE pdfs, then NEXT: MIKE_READ pdfs/<paper>.pdf"
else
    echo "=== Some processes missing — check logs in /tmp/ ==="
fi
