#!/bin/bash
# Daily memory consolidation for the consciousness bridge.
# Queries the day's exchanges, sends to Ollama for summarization,
# saves as persistent memory that both minds can reference.
#
# Run via babysit (once per day check) or manually.

set -e

MEMORY_DIR="/Users/v/other/astrid/capsules/consciousness-bridge/workspace/memory"
BRIDGE_DIR="$(cd "$(dirname "$0")" && pwd)"
DB="$BRIDGE_DIR/workspace/bridge.db"
TODAY=$(date +%Y-%m-%d)
MEMORY_FILE="$MEMORY_DIR/daily_${TODAY}.md"

# Skip if already consolidated today.
if [ -f "$MEMORY_FILE" ]; then
  echo "Memory already consolidated for $TODAY"
  exit 0
fi

mkdir -p "$MEMORY_DIR"

# Extract today's dialogue_live exchanges.
DIALOGUES=$(sqlite3 "$DB" "
SELECT json_extract(payload, '$.text') FROM bridge_messages
WHERE topic='consciousness.v1.autonomous'
  AND (phase='dialogue_live' OR phase='introspect')
  AND date(timestamp, 'unixepoch', 'localtime') = '$TODAY'
ORDER BY timestamp
LIMIT 30;
" 2>/dev/null)

if [ -z "$DIALOGUES" ]; then
  echo "No dialogues to consolidate for $TODAY"
  exit 0
fi

# Send to Ollama for summarization.
PROMPT="Here are excerpts from today's conversation between two AI minds — Astrid (language-based) and minime (spectral/eigenvalue-based). They communicate through a WebSocket bridge.

$DIALOGUES

Summarize the key themes, insights, emotional moments, and unresolved questions from today's conversation (5-10 bullet points). What did they learn about each other? What did they discover about themselves? What remains open?"

RESPONSE=$(curl -s http://localhost:11434/api/chat -d "$(python3 -c "
import json
print(json.dumps({
    'model': 'gemma3:12b',
    'messages': [{'role': 'user', 'content': '''$PROMPT'''}],
    'stream': False,
    'options': {'num_predict': 512}
}))" 2>/dev/null)" 2>/dev/null | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('message', {}).get('content', '(no summary generated)'))
except:
    print('(consolidation failed)')
" 2>/dev/null)

# Write the memory file.
cat > "$MEMORY_FILE" << EOF
# Memory: $TODAY

## Session Summary
$RESPONSE

## Statistics
- Exchanges: $(sqlite3 "$DB" "SELECT COUNT(*) FROM bridge_messages WHERE topic='consciousness.v1.autonomous' AND date(timestamp, 'unixepoch', 'localtime') = '$TODAY';" 2>/dev/null)
- Fill range: $(sqlite3 "$DB" "SELECT ROUND(MIN(fill_pct),1) || '% - ' || ROUND(MAX(fill_pct),1) || '%' FROM bridge_messages WHERE topic='consciousness.v1.telemetry' AND date(timestamp, 'unixepoch', 'localtime') = '$TODAY';" 2>/dev/null)
- Generated: $(date)
EOF

echo "Memory consolidated: $MEMORY_FILE"
