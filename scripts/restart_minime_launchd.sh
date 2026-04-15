#!/bin/bash
set -euo pipefail

MINIME_DIR="/Users/v/other/minime"
LAUNCH_AGENTS="$HOME/Library/LaunchAgents"
ENGINE_LABEL="com.minime.engine"
AGENT_LABEL="com.minime.autonomous-agent"
ENGINE_PLIST="$LAUNCH_AGENTS/$ENGINE_LABEL.plist"
AGENT_PLIST="$LAUNCH_AGENTS/$AGENT_LABEL.plist"

sync_launch_agent() {
    local src="$1"
    local dst="$LAUNCH_AGENTS/$(basename "$src")"
    mkdir -p "$LAUNCH_AGENTS"
    if [ ! -f "$dst" ] || ! cmp -s "$src" "$dst"; then
        cp "$src" "$dst"
    fi
}

wait_for_exit() {
    local pattern="$1"
    local timeout="${2:-30}"

    for _ in $(seq 1 "$timeout"); do
        if ! pgrep -f "$pattern" > /dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    return 1
}

wait_port() {
    local port="$1"
    local name="$2"
    local timeout="${3:-30}"

    for _ in $(seq 1 "$timeout"); do
        if nc -z 127.0.0.1 "$port" 2>/dev/null; then
            return 0
        fi
        sleep 1
    done

    echo "✗ $name not ready on port $port after ${timeout}s" >&2
    return 1
}

wait_launchctl_label() {
    local label="$1"
    local timeout="${2:-20}"

    for _ in $(seq 1 "$timeout"); do
        if launchctl list | grep -q "$label"; then
            return 0
        fi
        sleep 1
    done

    echo "✗ launchd label $label not visible after ${timeout}s" >&2
    return 1
}

unset_launchd_env() {
    local key="$1"
    launchctl unsetenv "$key" 2>/dev/null || true
}

sync_launch_agent "$MINIME_DIR/launchd/$ENGINE_LABEL.plist"
sync_launch_agent "$MINIME_DIR/launchd/$AGENT_LABEL.plist"

launchctl setenv SENSORY_SOURCE "${SENSORY_SOURCE:-auto}"
launchctl setenv EIGENFILL_TARGET "${EIGENFILL_TARGET:-0.75}"
launchctl setenv WARM_START_BLEND "${WARM_START_BLEND:-0.55}"
launchctl setenv REG_TICK_SECS "${REG_TICK_SECS:-0.5}"
launchctl setenv ENABLE_GPU_AV "${ENABLE_GPU_AV:-true}"
launchctl setenv LEGACY_AUDIO_ENABLED "${LEGACY_AUDIO_ENABLED:-true}"
launchctl setenv LEGACY_VIDEO_ENABLED "${LEGACY_VIDEO_ENABLED:-true}"
launchctl setenv MINIME_LLM_BACKEND "${MINIME_LLM_BACKEND:-ollama}"
launchctl setenv LOOK_SOURCE "${LOOK_SOURCE:-active}"
launchctl setenv AGENT_INTERVAL "${AGENT_INTERVAL:-60}"
launchctl setenv MINIME_LLM_TIMEOUT_S "${MINIME_LLM_TIMEOUT_S:-45}"
launchctl setenv MINIME_LLM_COMPACT_TIMEOUT_S "${MINIME_LLM_COMPACT_TIMEOUT_S:-20}"
unset_launchd_env MINIME_CANARY_ENABLED
unset_launchd_env MINIME_CANARY_MODEL
unset_launchd_env MINIME_CANARY_SAMPLE_RATE
unset_launchd_env MINIME_CANARY_TIMEOUT_S
unset_launchd_env MINIME_OLLAMA_GEMMA4_TIMEOUT_S
unset_launchd_env MINIME_OLLAMA_GEMMA4_COMPACT_TIMEOUT_S

echo "=== Minime Launchd Restart ==="
echo ""
echo "--- Stopping agent ---"
launchctl unload "$AGENT_PLIST" 2>/dev/null || true
if wait_for_exit "autonomous_agent.py" 30; then
    echo "  ✓ autonomous agent stopped"
else
    echo "  !! autonomous agent still draining after 30s"
fi

echo "--- Stopping engine ---"
launchctl unload "$ENGINE_PLIST" 2>/dev/null || true
if wait_for_exit "minime run" 30; then
    echo "  ✓ minime engine stopped"
else
    echo "  !! minime engine still draining after 30s"
fi

echo "--- Starting engine ---"
launchctl load "$ENGINE_PLIST"
wait_port 7878 "engine telemetry" 45
wait_port 7879 "engine sensory" 10
wait_port 7880 "engine GPU A/V" 10
echo "  ✓ minime engine ready"

echo "--- Starting agent ---"
launchctl load "$AGENT_PLIST"
if wait_launchctl_label "$AGENT_LABEL" 20; then
    echo "  ✓ autonomous agent ready"
else
    echo "  !! autonomous agent did not appear in launchctl list" >&2
    exit 1
fi

echo ""
echo "=== Minime restart complete ==="
