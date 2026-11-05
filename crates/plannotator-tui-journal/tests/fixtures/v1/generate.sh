#!/usr/bin/env bash
# Write this folder's fixtures with Plannotator's own CLI: a review of a session's last
# messages, in a data directory of its own, round 1 sent and answered, then round 2 under way.
#
# Usage: generate.sh <plannotator>   (a build with `plannotator threads`)
set -euo pipefail
PLANNOTATOR="$1"
HERE="$(cd "$(dirname "$0")" && pwd)"
DATA="$(mktemp -d)"
trap 'rm -rf "$DATA"' EXIT
export PLANNOTATOR_DATA_DIR="$DATA"
for name in $(env | sed -n 's/^\(PLANNOTATOR_[A-Za-z0-9_]*\)=.*/\1/p'); do
  [ "$name" = PLANNOTATOR_DATA_DIR ] || unset "$name"
done
T='"target":{"kind":"last","host":"claude-code","session":"ses_a"}'
threads() { "$PLANNOTATOR" threads "$1" "$2"; }

threads comment "{$T,\"id\":\"c1\",\"on\":{\"kind\":\"message\",\"message_id\":\"m1\"},\"quote\":\"turns the new checkout off\",\"text\":\"Who watches at night?\"}" >/dev/null
threads comment "{$T,\"id\":\"c2\",\"on\":{\"kind\":\"message\",\"message_id\":\"m1\"},\"quote\":\"We ship on Friday.\",\"text\":\"\",\"type\":\"deletion\"}" >/dev/null
threads send "{$T,\"mode\":\"round\",\"idempotency_key\":\"press-1\"}" >"$HERE/send.json"
send=$(sed -n 's/.*"send":{"id":"\(snd_[0-9A-Z]*\)".*/\1/p' "$HERE/send.json")
threads delivered "{$T,\"sends\":[\"$send\"]}" >/dev/null
review=$(sed -n 's/^{"review":"\([a-z0-9-]*\)".*/\1/p' "$HERE/send.json")
"$PLANNOTATOR" reply --review "$review" c1 'The on-call engineer, from the runbook.' >/dev/null
"$PLANNOTATOR" reply --review "$review" c2 'Removed.' >/dev/null
threads resolve "{$T,\"id\":\"c2\"}" >/dev/null
# The agent's first reply: the one under c1.
agent=$("$PLANNOTATOR" threads --review "$review" --json | grep -o '"id":"rep_[0-9A-Z]*"' | head -1 | cut -d'"' -f4)
threads comment "{$T,\"id\":\"s1\",\"parent\":\"$agent\",\"quote\":\"the runbook\",\"text\":\"Which runbook?\"}" >/dev/null
threads comment "{$T,\"id\":\"r1\",\"re\":\"c1\",\"text\":\"And on weekends?\"}" >/dev/null
threads comment "{$T,\"id\":\"d1\",\"on\":{\"kind\":\"message\",\"message_id\":\"m2\"},\"quote\":\"Second reply.\",\"text\":\"Shorter.\"}" >/dev/null
threads view "{$T}" >"$HERE/view.json"
echo "wrote send.json and view.json with $("$PLANNOTATOR" --version | head -1)"
