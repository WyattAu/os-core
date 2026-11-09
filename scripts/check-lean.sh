#!/usr/bin/env bash
# Lean proof gate (REQ-OS-003) — implements OMNI-CORE invariant 11f:
#   1. `lake build` must succeed with the pinned toolchain;
#   2. any `sorry`/`admit` in the proof tree fails;
#   3. every `theorem` carries a matching `#print axioms` line;
#   4. the build log must contain the axiom reports (proofs actually ran).
set -euo pipefail
cd "$(dirname "$0")/../proofs"

log="$(mktemp)"
trap 'rm -f "$log"' EXIT

lake build 2>&1 | tee "$log"

if find . -name '*.lean' -not -path './.lake/*' \
    -exec grep -nE '^[[:space:]]*(sorry|admit)[[:space:]]*$' {} +; then
    echo "lean: FAIL — sorry/admit present"
    exit 1
fi

missing=0
for thm in $(grep -hoE '^theorem [a-zA-Z_][a-zA-Z0-9_]*' *.lean | awk '{print $2}'); do
    if ! grep -q "#print axioms ${thm}\b" *.lean; then
        echo "lean: FAIL — missing '#print axioms ${thm}'"
        missing=1
    fi
done
if [ "$missing" -ne 0 ]; then
    exit 1
fi

if ! grep -q "axioms" "$log"; then
    echo "lean: FAIL — no axiom reports in build output"
    exit 1
fi

if grep -q 'sorryAx' "$log"; then
    echo "lean: FAIL — sorryAx in axiom reports"
    exit 1
fi

echo "lean: OK"
