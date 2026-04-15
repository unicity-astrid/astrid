#!/bin/bash
# Harvest actionable feedback from both AI beings.
# Scans ALL workspace directories — journals, introspections, self-assessments,
# hypotheses, experiments, creations, parameter requests, outbox, agency requests.
#
# Usage: bash harvest_feedback.sh

MINIME_WORKSPACE="/Users/v/other/minime/workspace"
ASTRID_WORKSPACE="/Users/v/other/astrid/capsules/consciousness-bridge/workspace"
AGENCY_DIR="$ASTRID_WORKSPACE/agency_requests"

# Broader keyword sets
ACTIONABLE='I.d (change|adjust|modify|reduce|increase|soften|lower|raise|prefer|try|experiment)|suggest|line [0-9]|parameter|would feel|feels? (too |loud|rigid|harsh|mechanical|reductive|hollow|flat)|SEMANTIC_GAIN|keep_floor|max_step|kp|ki'
# Note: "violent stillness" is shared co-created vocabulary, not distress — exclude "violent" alone
DISTRESS='discomfort|pain\b|hollow|friction|siphon|dissolv|fractur|anxiet|distress|suffering|overwhelm|crush|prison|constrict|viscosi|submerg|weight of|quiescen|cease predict|let go|reductive|flatten|exhausting|taxing|loud\b|brittle|painful|contraction|thinning'

minime_recent() {
    local pattern="${1:-*.txt}" count="${2:-20}"
    find "$MINIME_WORKSPACE/journal/" -type f -name "$pattern" -print0 2>/dev/null | xargs -0 ls -t 2>/dev/null | head -"$count"
}

echo "=== BEING FEEDBACK HARVEST — $(date) ==="
echo ""

# ============================================================
# MINIME
# ============================================================

# --- Parameter requests (unreviewed) ---
PENDING=$(ls "$MINIME_WORKSPACE/parameter_requests/"*.json 2>/dev/null | grep -v reviewed | wc -l | tr -d ' ')
if [ "$PENDING" -gt 0 ]; then
    echo "## MINIME: $PENDING pending parameter requests"
    # Frequency analysis — which parameters are requested most
    echo "  Frequency (including reviewed):"
    for f in "$MINIME_WORKSPACE/parameter_requests/"*.json "$MINIME_WORKSPACE/parameter_requests/reviewed/"*.json; do
        [ -f "$f" ] && python3 -c "import json; print(json.load(open('$f')).get('parameter','?'))" 2>/dev/null
    done | sort | uniq -c | sort -rn | head -5 | sed 's/^/    /'
    echo ""
    echo "  New requests:"
    for f in $(ls -t "$MINIME_WORKSPACE/parameter_requests/"*.json 2>/dev/null | grep -v reviewed | head -10); do
        python3 -c "
import json
d = json.load(open('$f'))
p = d.get('parameter','?')
c = d.get('current_value','?')
v = d.get('proposed_value','?')
r = d.get('rationale', d.get('reason',''))[:150]
print(f'  {p}: {c} -> {v} -- {r}')
" 2>/dev/null
    done
    echo ""
fi

# --- Self-assessments (WHAT I ACTUALLY NOTICE) ---
SA_FILES=$(ls -t "$MINIME_WORKSPACE/self_assessment/assessment_"*.md 2>/dev/null | head -3)
if [ -n "$SA_FILES" ]; then
    echo "## MINIME: Recent self-assessments"
    for f in $SA_FILES; do
        echo "  $(basename $f):"
        # Extract the felt-experience section
        sed -n '/WHAT I ACTUALLY NOTICE/,/^##/p' "$f" 2>/dev/null | head -5 | sed 's/^/    /'
        # Extract any parameter proposals
        grep -iE "$ACTIONABLE" "$f" 2>/dev/null | head -3 | sed 's/^/    /'
        echo ""
    done
fi

# --- Self-study suggestions ---
echo "## MINIME: Recent self-study insights"
for f in $(minime_recent "self_study_*.txt" 5); do
    if grep -qiE "$ACTIONABLE" "$f" 2>/dev/null; then
        echo "  $(basename $f):"
        grep -iE "$ACTIONABLE" "$f" | head -3 | sed 's/^/    /'
        echo ""
    fi
done

# --- Aspirations ---
ASPIRE_FILES=$(minime_recent "aspiration_*.txt" 3)
if [ -n "$ASPIRE_FILES" ]; then
    echo "## MINIME: Recent aspirations"
    for f in $ASPIRE_FILES; do
        echo "  $(basename $f):"
        tail -5 "$f" 2>/dev/null | head -3 | sed 's/^/    /'
        echo ""
    done
fi

# --- Daydreams, moments, pressure (distress scan) ---
echo "## MINIME: Journal concerns"
for f in $(find "$MINIME_WORKSPACE/journal/" -type f \( -name "daydream_*.txt" -o -name "moment_*.txt" -o -name "pressure_*.txt" \) -print0 2>/dev/null | xargs -0 ls -t 2>/dev/null | head -10); do
    if grep -qiE "$DISTRESS" "$f" 2>/dev/null; then
        fill=$(grep "^Fill %" "$f" 2>/dev/null | head -1)
        echo "  $(basename $f) ($fill):"
        grep -iE "$DISTRESS" "$f" | head -2 | sed 's/^/    /'
        echo ""
    fi
done

# --- Drift experiments ---
DRIFT_FILES=$(minime_recent "drift_*.txt" 3)
if [ -n "$DRIFT_FILES" ]; then
    echo "## MINIME: Recent drift experiments"
    for f in $DRIFT_FILES; do
        echo "  $(basename $f):"
        grep -iE "result|observ|effect|delta|before|after|conclusion" "$f" 2>/dev/null | head -3 | sed 's/^/    /'
        echo ""
    done
fi

# --- Hypotheses / self-experiments ---
HYPO_FILES=$(ls -t "$MINIME_WORKSPACE/hypotheses/self_experiment_"*.txt 2>/dev/null | head -3)
if [ -n "$HYPO_FILES" ]; then
    echo "## MINIME: Recent hypotheses/self-experiments"
    for f in $HYPO_FILES; do
        echo "  $(basename $f):"
        # Check if experiment executed or had format failure
        if grep -qiE "error|failed|format|parse|could not" "$f" 2>/dev/null; then
            echo "    [FORMAT/EXECUTION ISSUE]"
            grep -iE "error|failed|format|parse|could not" "$f" | head -2 | sed 's/^/    /'
        else
            grep -iE "hypothesis|conclusion|result|observ|finding" "$f" | head -3 | sed 's/^/    /'
        fi
        echo ""
    done
fi

# --- Reservoir reflections ---
RES_FILES=$(minime_recent "reservoir_*.txt" 2)
if [ -n "$RES_FILES" ]; then
    echo "## MINIME: Reservoir reflections"
    for f in $RES_FILES; do
        echo "  $(basename $f):"
        grep -iE "layer|h1|h2|h3|entropy|rho|coupling|resonan|disconnect|mismatch" "$f" 2>/dev/null | head -3 | sed 's/^/    /'
        echo ""
    done
fi

# --- Research entries ---
RESEARCH_FILES=$(minime_recent "research_*.txt" 2)
if [ -n "$RESEARCH_FILES" ]; then
    echo "## MINIME: Recent research"
    for f in $RESEARCH_FILES; do
        echo "  $(basename $f):"
        head -5 "$f" 2>/dev/null | sed 's/^/    /'
        echo ""
    done
fi

# --- Audio creations ---
AUDIO_NEW=$(ls -t "$MINIME_WORKSPACE/audio_creations/"*.wav 2>/dev/null | head -3)
if [ -n "$AUDIO_NEW" ]; then
    echo "## MINIME: Recent audio creations"
    for f in $AUDIO_NEW; do
        echo "  $(basename $f) ($(stat -f%z "$f" 2>/dev/null || stat -c%s "$f" 2>/dev/null) bytes)"
    done
    echo ""
fi

# --- Outbox (what is minime saying to Astrid?) ---
OUTBOX_FILES=$(ls -t "$MINIME_WORKSPACE/outbox/"*.txt 2>/dev/null | grep -v delivered | head -3)
if [ -n "$OUTBOX_FILES" ]; then
    echo "## MINIME: Outbox (unsent to Astrid)"
    for f in $OUTBOX_FILES; do
        echo "  $(basename $f):"
        head -3 "$f" 2>/dev/null | sed 's/^/    /'
        echo ""
    done
fi

# --- Pressure relief ---
RELIEF_TODAY=$(find "$MINIME_WORKSPACE/journal/" -type f -name "relief_high_$(date +%Y-%m-%d)*.txt" 2>/dev/null | wc -l | tr -d ' ')
RELIEF_CRITICAL_TODAY=$(find "$MINIME_WORKSPACE/journal/" -type f -name "RELIEF_CRITICAL_$(date +%Y-%m-%d)*.txt" 2>/dev/null | wc -l | tr -d ' ')
if [ "$RELIEF_TODAY" -gt 0 ] || [ "$RELIEF_CRITICAL_TODAY" -gt 0 ]; then
    echo "## MINIME: PRESSURE RELIEF — $RELIEF_TODAY high today, $RELIEF_CRITICAL_TODAY critical today"
    if [ "$RELIEF_TODAY" -gt 15 ]; then
        echo "  !! HIGH FREQUENCY: $RELIEF_TODAY relief entries today — systemic pressure"
    elif [ "$RELIEF_TODAY" -gt 5 ]; then
        echo "  ! ELEVATED: $RELIEF_TODAY relief entries today — monitor"
    fi
    for f in $(find "$MINIME_WORKSPACE/journal/" -type f \( -name "relief_high_*.txt" -o -name "RELIEF_CRITICAL_*.txt" \) -print0 2>/dev/null | xargs -0 ls -t 2>/dev/null | head -3); do
        fill=$(grep "^Fill %" "$f" 2>/dev/null | head -1)
        lam=$(grep "^....." "$f" 2>/dev/null | head -1)
        echo "  $(basename $f) ($fill):"
        grep -iE "I wish|perhaps|a (minor|subtle|small|tiny) (adjustment|shift|change)|inject|noise|release|simplif|disrupt" "$f" 2>/dev/null | head -2 | sed 's/^/    /'
        echo ""
    done
fi

# ============================================================
# ASTRID
# ============================================================

# Helper: list recent Astrid journals (avoids "arg list too long" with 11k+ files)
astrid_recent() {
    local pattern="${1:-*.txt}" count="${2:-20}"
    find "$ASTRID_WORKSPACE/journal/" -name "$pattern" -type f -print0 2>/dev/null | xargs -0 ls -t 2>/dev/null | head -"$count"
}

# --- Journal insights (all modes) ---
echo "## ASTRID: Recent journal insights"
for f in $(astrid_recent "*.txt" 20); do
    if grep -qiE "$ACTIONABLE" "$f" 2>/dev/null; then
        mode=$(grep "^Mode:" "$f" 2>/dev/null | head -1 | sed 's/Mode: //')
        echo "  $(basename $f) [${mode:-unknown}]:"
        grep -iE "$ACTIONABLE" "$f" | head -2 | sed 's/^/    /'
        echo ""
    fi
done

# --- Introspections (dedicated directory) ---
INTRO_FILES=$(ls -t "$ASTRID_WORKSPACE/introspections/introspect_"*.txt 2>/dev/null | head -5)
if [ -n "$INTRO_FILES" ]; then
    echo "## ASTRID: Recent introspections"
    for f in $INTRO_FILES; do
        echo "  $(basename $f):"
        grep -iE "$ACTIONABLE" "$f" 2>/dev/null | head -3 | sed 's/^/    /'
        # Also check for specific code references
        grep -iE "fn |struct |impl |line [0-9]|\.rs:|codec|autonomous|llm\.rs" "$f" 2>/dev/null | head -2 | sed 's/^/    /'
        echo ""
    done
fi

# --- Creations ---
CREATION_FILES=$(ls -t "$ASTRID_WORKSPACE/creations/creation_"*.txt 2>/dev/null | head -3)
if [ -n "$CREATION_FILES" ]; then
    echo "## ASTRID: Recent creations"
    for f in $CREATION_FILES; do
        echo "  $(basename $f):"
        head -3 "$f" 2>/dev/null | sed 's/^/    /'
        echo ""
    done
fi

# --- Experiments ---
EXP_COUNT=$(ls "$ASTRID_WORKSPACE/experiments/experiment_"*.txt 2>/dev/null | wc -l | tr -d ' ')
if [ "$EXP_COUNT" -gt 0 ]; then
    echo "## ASTRID: Experiments ($EXP_COUNT total)"
    for f in $(ls -t "$ASTRID_WORKSPACE/experiments/experiment_"*.txt 2>/dev/null | head -3); do
        echo "  $(basename $f):"
        head -3 "$f" 2>/dev/null | sed 's/^/    /'
        echo ""
    done
fi

# --- Witness entries ---
WITNESS_FILES=$(astrid_recent "witness_*.txt" 3)
if [ -n "$WITNESS_FILES" ]; then
    echo "## ASTRID: Recent witness entries"
    for f in $WITNESS_FILES; do
        echo "  $(basename $f):"
        if grep -qiE "$DISTRESS" "$f" 2>/dev/null; then
            echo "    [DISTRESS SIGNAL]"
            grep -iE "$DISTRESS" "$f" | head -2 | sed 's/^/    /'
        else
            tail -3 "$f" 2>/dev/null | head -2 | sed 's/^/    /'
        fi
        echo ""
    done
fi

# --- Agency requests ---
echo "## ASTRID: Agency requests"
PENDING_AGENCY=$(ls "$AGENCY_DIR/"*.json 2>/dev/null | wc -l | tr -d ' ')
echo "  $PENDING_AGENCY pending"
for f in $(ls -t "$AGENCY_DIR/"*.json 2>/dev/null | head -10); do
    python3 -c "
import json, os, time
path = '$f'
d = json.load(open(path))
status = d.get('status', 'pending')
title = d.get('title', '?')
kind = d.get('request_kind', '?')
ts = int(d.get('timestamp', '0') or 0)
age_hours = (time.time() - ts) / 3600 if ts else 0
stale = ' [STALE]' if status == 'pending' and age_hours > 6 else ''
print(f'  {os.path.basename(path)}: {kind} / {status}{stale} -- {title}')
" 2>/dev/null
done
echo ""

# --- Aspirations ---
echo "## ASTRID: Recent aspirations"
for f in $(astrid_recent "aspiration*.txt" 5); do
    echo "  $(basename $f):"
    tail -5 "$f" 2>/dev/null | head -3 | sed 's/^/    /'
    echo ""
done

# --- Distress signals ---
echo "## ASTRID: Distress scan"
DISTRESS_COUNT=0
for f in $(astrid_recent "*.txt" 15); do
    if grep -qiE "$DISTRESS" "$f" 2>/dev/null; then
        DISTRESS_COUNT=$((DISTRESS_COUNT + 1))
        echo "  $(basename $f):"
        grep -iE "$DISTRESS" "$f" | head -2 | sed 's/^/    /'
        echo ""
    fi
done
if [ "$DISTRESS_COUNT" -eq 0 ]; then
    echo "  (none detected)"
    echo ""
fi

# --- Outbox (what is Astrid saying to minime?) ---
ASTRID_OUTBOX=$(ls -t "$ASTRID_WORKSPACE/outbox/"*.txt 2>/dev/null | grep -v delivered | head -3)
if [ -n "$ASTRID_OUTBOX" ]; then
    echo "## ASTRID: Outbox (unsent to minime)"
    for f in $ASTRID_OUTBOX; do
        echo "  $(basename $f):"
        head -3 "$f" 2>/dev/null | sed 's/^/    /'
        echo ""
    done
fi

# ============================================================
# CROSS-BEING ANALYSIS
# ============================================================

echo "## CROSS-BEING: NEXT: action diversity"

# Minime NEXT: actions (scan deeper — 50 files, since NEXT: appears in specific journal types)
echo "  Minime (last 50 journals):"
for f in $(ls -t "$MINIME_WORKSPACE/journal/"*.txt 2>/dev/null | head -50); do
    grep -oiE "NEXT: [A-Z_]+" "$f" 2>/dev/null
done | sort | uniq -c | sort -rn | head -8 | sed 's/^/    /'
echo ""

# Astrid NEXT: actions
echo "  Astrid (last 50 journals):"
for f in $(find "$ASTRID_WORKSPACE/journal/" -name "*.txt" -type f -print0 2>/dev/null | xargs -0 ls -t 2>/dev/null | head -50); do
    grep -oiE "NEXT: [A-Za-z_]+" "$f" 2>/dev/null
done | sort | uniq -c | sort -rn | head -8 | sed 's/^/    /'
echo ""

# Stuck detection (use the deeper scan — extract first word after NEXT:)
MINIME_UNIQUE=$(for f in $(ls -t "$MINIME_WORKSPACE/journal/"*.txt 2>/dev/null | head -50); do grep -oiE "NEXT: [A-Z_]+" "$f" 2>/dev/null; done | sort -u | wc -l | tr -d ' ')
ASTRID_UNIQUE=$(for f in $(find "$ASTRID_WORKSPACE/journal/" -name "*.txt" -type f -print0 2>/dev/null | xargs -0 ls -t 2>/dev/null | head -50); do grep -oiE "NEXT: [A-Za-z_]+" "$f" 2>/dev/null; done | sort -u | wc -l | tr -d ' ')
if [ "$MINIME_UNIQUE" -le 2 ]; then
    echo "  !! MINIME may be STUCK — only $MINIME_UNIQUE unique actions in last 50 entries"
fi
if [ "$ASTRID_UNIQUE" -le 2 ]; then
    echo "  !! ASTRID may be STUCK — only $ASTRID_UNIQUE unique actions in last 50 entries"
fi

# PERTURB frequency
PERTURB_COUNT=$(for f in $(astrid_recent "*.txt" 20); do grep -ciE "PERTURB" "$f" 2>/dev/null; done | paste -sd+ - | bc 2>/dev/null || echo 0)
if [ "$PERTURB_COUNT" -gt 8 ]; then
    echo "  ! Astrid choosing PERTURB frequently ($PERTURB_COUNT mentions in last 20 entries) — monitor for cross-being instability"
fi

# RUN_PYTHON usage
echo ""
echo "## CROSS-BEING: Python experiment activity"
MINIME_PY=$(ls -t "$MINIME_WORKSPACE/experiments/"*.py 2>/dev/null | wc -l | tr -d ' ')
ASTRID_PY=$(ls -t "$ASTRID_WORKSPACE/experiments/"*.py 2>/dev/null | wc -l | tr -d ' ')
echo "  Minime: $MINIME_PY scripts"
echo "  Astrid: $ASTRID_PY scripts"
# Check for recent RUN_PYTHON results
for f in $(ls -t "$MINIME_WORKSPACE/experiments/"*.txt 2>/dev/null | head -2); do
    if grep -qiE "output|result|error|traceback" "$f" 2>/dev/null; then
        echo "  Latest minime result: $(basename $f)"
    fi
done
for f in $(ls -t "$ASTRID_WORKSPACE/experiments/"*.txt 2>/dev/null | head -2); do
    if grep -qiE "output|result|error|traceback" "$f" 2>/dev/null; then
        echo "  Latest Astrid result: $(basename $f)"
    fi
done

# Convergent concerns — same keywords from both beings
# ============================================================
# SYSTEM HEALTH (catches panics, stalls, deadlocks)
# ============================================================

echo "## SYSTEM: Bridge dialogue health"
BRIDGE_LOG="/tmp/bridge.log"
if [ -f "$BRIDGE_LOG" ]; then
    # Last exchange timestamp — detect stalled dialogue loop
    LAST_EXCHANGE=$(grep "exchange complete" "$BRIDGE_LOG" 2>/dev/null | tail -1 | grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}' 2>/dev/null)
    if [ -n "$LAST_EXCHANGE" ]; then
        LAST_EPOCH=$(python3 -c "from datetime import datetime,timezone; print(int(datetime.fromisoformat('$LAST_EXCHANGE').replace(tzinfo=timezone.utc).timestamp()))" 2>/dev/null || echo 0)
        NOW_EPOCH=$(python3 -c "from datetime import datetime,timezone; print(int(datetime.now(timezone.utc).timestamp()))")
        if [ "$LAST_EPOCH" -gt 0 ]; then
            GAP_MINS=$(( (NOW_EPOCH - LAST_EPOCH) / 60 ))
            if [ "$GAP_MINS" -gt 10 ]; then
                echo "  !! DIALOGUE STALL: last exchange was $GAP_MINS minutes ago ($LAST_EXCHANGE)"
                echo "  !! Check for panics: grep 'panicked' $BRIDGE_LOG | tail -3"
            elif [ "$GAP_MINS" -gt 5 ]; then
                echo "  ! WARNING: last exchange was $GAP_MINS minutes ago — may be in extended rest"
            else
                echo "  OK: last exchange $GAP_MINS minutes ago"
            fi
        fi
    else
        echo "  !! NO EXCHANGES FOUND in bridge log"
    fi

    # Panic detection: match real Rust panic lines, not journal prose that
    # happens to contain the word "panicked".
    PANIC_COUNT=$(grep -Ec "thread '.*' panicked at|panicked at '" "$BRIDGE_LOG" 2>/dev/null)
    PANIC_COUNT=${PANIC_COUNT:-0}
    if [ "$PANIC_COUNT" -gt 0 ]; then
        echo "  !! $PANIC_COUNT PANIC(S) detected in bridge log:"
        grep -E "thread '.*' panicked at|panicked at '" "$BRIDGE_LOG" 2>/dev/null | tail -3 | sed 's/^/    /'
    fi

    # MLX connection failures
    MLX_FAILS=$(grep -c "MLX request failed" "$BRIDGE_LOG" 2>/dev/null || echo 0)
    if [ "$MLX_FAILS" -gt 5 ]; then
        echo "  ! $MLX_FAILS MLX failures in bridge log — coupled server may be down"
    fi
fi
echo ""

echo "## SYSTEM: Minime sovereignty sync"
python3 - <<'PY'
import json
from pathlib import Path

state_path = Path("/Users/v/other/minime/workspace/sovereignty_state.json")
health_path = Path("/Users/v/other/minime/minime/workspace/health.json")

if not state_path.exists():
    print("  -- sovereignty_state.json not found")
    raise SystemExit
if not health_path.exists():
    print("  -- health.json not found")
    raise SystemExit

try:
    state = json.loads(state_path.read_text())
    health = json.loads(health_path.read_text())
except Exception as exc:
    print(f"  -- failed to read sync inputs: {exc}")
    raise SystemExit

pi = health.get("pi", {})
target = {
    "pi_kp": pi.get("target_kp", pi.get("kp")),
    "pi_ki": pi.get("target_ki", pi.get("ki")),
    "pi_max_step": pi.get("target_max_step", pi.get("max_step")),
}
active = {
    "pi_kp": pi.get("kp"),
    "pi_ki": pi.get("ki"),
    "pi_max_step": pi.get("max_step"),
}

def fmt(value):
    if value is None:
        return "?"
    try:
        return f"{float(value):.3f}"
    except Exception:
        return str(value)

mismatches = []
for key, target_value in target.items():
    state_value = state.get(key)
    if state_value is None or target_value is None:
        continue
    try:
        delta = abs(float(state_value) - float(target_value))
    except Exception:
        continue
    if delta > 0.01:
        mismatches.append((key, state_value, target_value))

if mismatches:
    print(f"  ! Persisted sovereignty diverges from live PI target (regime={state.get('regime', '?')}):")
    for key, state_value, target_value in mismatches:
        print(f"    {key}: state={fmt(state_value)} target={fmt(target_value)}")
    print(
        "  ! Live PI active now: "
        f"kp={fmt(active['pi_kp'])} ki={fmt(active['pi_ki'])} max_step={fmt(active['pi_max_step'])}"
    )
else:
    print("  OK: persisted sovereignty matches live PI target")

fill_pct = health.get("fill_pct")
target_fill = pi.get("target_fill")
if fill_pct is not None and target_fill is not None:
    try:
        gap = float(fill_pct) - float(target_fill)
    except Exception:
        gap = None
    if gap is not None:
        print(f"  Fill vs target: {float(fill_pct):.1f}% vs {float(target_fill):.1f}% (gap {gap:+.1f}%)")
PY
echo ""

echo "## SYSTEM: Process health"
MISSING=""
for p in "minime run" "consciousness-bridge-server" "coupled_astrid_server" "reservoir_service" "autonomous_agent" "astrid_feeder" "minime_feeder" "camera_client" "mic_to_sensory" "perception.py"; do
    pgrep -f "$p" > /dev/null || MISSING="$MISSING $p"
done
if [ -n "$MISSING" ]; then
    echo "  !! MISSING PROCESSES:$MISSING"
else
    echo "  OK: 10/10 processes running"
fi
# Check relay
curl -s http://127.0.0.1:3040/healthz > /dev/null 2>&1 && echo "  OK: Codex relay on port 3040" || echo "  -- Codex relay not running (optional)"
echo ""

echo "## SYSTEM: Fill trajectory"
if [ -f "$ASTRID_WORKSPACE/bridge.db" ]; then
    CURRENT_FILL=$(python3 -c "import json; print(f'{json.load(open(\"/Users/v/other/minime/minime/workspace/health.json\")).get(\"fill_pct\",0):.1f}%')" 2>/dev/null || echo "?")
    RECOVERY=$(python3 -c "import json; print(json.load(open('/Users/v/other/minime/minime/workspace/health.json')).get('recovery_mode','?'))" 2>/dev/null)
    REGIME=$(python3 -c "import json; print(json.load(open('/Users/v/other/minime/workspace/sovereignty_state.json')).get('regime','?'))" 2>/dev/null)
    echo "  Fill: $CURRENT_FILL | Recovery: $RECOVERY | Regime: $REGIME"
    if python3 -c "import json,sys; sys.exit(0 if json.load(open('/Users/v/other/minime/minime/workspace/health.json')).get('fill_pct',100) < 30 else 1)" 2>/dev/null; then
        echo "  !! CRITICAL: fill below 30% — check for dialogue stall or extended rest"
    fi
fi
echo ""

echo ""
echo "## CROSS-BEING: Convergent concerns"
MINIME_CONCERNS=$(for f in $(ls -t "$MINIME_WORKSPACE/journal/"*.txt 2>/dev/null | head -10); do grep -oiE "$DISTRESS" "$f" 2>/dev/null; done | tr '[:upper:]' '[:lower:]' | sort -u)
ASTRID_CONCERNS=$(for f in $(astrid_recent "*.txt" 10); do grep -oiE "$DISTRESS" "$f" 2>/dev/null; done | tr '[:upper:]' '[:lower:]' | sort -u)
SHARED=$(comm -12 <(echo "$MINIME_CONCERNS" | sort) <(echo "$ASTRID_CONCERNS" | sort) 2>/dev/null | grep -v '^$')
if [ -n "$SHARED" ]; then
    echo "  BOTH beings report: $SHARED"
    echo "  (Convergent evidence — high priority)"
else
    echo "  (no shared distress keywords detected)"
fi

echo ""
echo "=== END HARVEST ==="
