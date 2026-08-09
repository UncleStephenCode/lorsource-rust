#!/usr/bin/env bash
set -euo pipefail

sRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${ORIGINAL_ROOT:?Set ORIGINAL_ROOT to the checked-out Java source tree}"

sWebapp="$ORIGINAL_ROOT/src/main/webapp"
sBuiltWebapp="${JAVA_WEBAPP_BUILD_DIR:-$ORIGINAL_ROOT/target/lor-1.0-SNAPSHOT}"

for sSource in \
  "$sBuiltWebapp/js/lor.js" \
  "$sBuiltWebapp/js/plugins.js" \
  "$sBuiltWebapp/js/diff_match_patch.js" \
  "$sWebapp/js/lor_view_diff_history.js" \
  "$sWebapp/manifest.json" \
  "$sWebapp/robots.txt" \
  "$sWebapp/googlea3fb422736ed276d.html" \
  "$sBuiltWebapp/qrerror/combined.css" \
  "$sBuiltWebapp/qrerror/good-penguin.png"; do
  if [[ ! -f "$sSource" ]]; then
    echo "Required Java runtime asset is missing: $sSource" >&2
    echo "Build the original webapp first (for example with scripts/run-java-parity-runtime.sh start)." >&2
    exit 1
  fi
done

mkdir -p "$sRoot/static/js" "$sRoot/static/qrerror"
install -m 0644 "$sBuiltWebapp/js/lor.js" "$sRoot/static/js/lor.js"
install -m 0644 "$sBuiltWebapp/js/plugins.js" "$sRoot/static/js/plugins.js"
install -m 0644 "$sBuiltWebapp/js/diff_match_patch.js" "$sRoot/static/js/diff_match_patch.js"
install -m 0644 "$sWebapp/js/lor_view_diff_history.js" "$sRoot/static/js/lor_view_diff_history.js"
install -m 0644 "$sWebapp/manifest.json" "$sRoot/static/manifest.json"
install -m 0644 "$sWebapp/robots.txt" "$sRoot/static/robots.txt"
install -m 0644 "$sWebapp/googlea3fb422736ed276d.html" "$sRoot/static/googlea3fb422736ed276d.html"
install -m 0644 "$sBuiltWebapp/qrerror/combined.css" "$sRoot/static/qrerror/combined.css"
install -m 0644 "$sBuiltWebapp/qrerror/good-penguin.png" "$sRoot/static/qrerror/good-penguin.png"

echo "Java runtime browser assets synchronized from $ORIGINAL_ROOT"
