#!/usr/bin/env bash
# Runs a cargo command; on failure, re-emits compiler errors as GitHub annotations
# so they are visible in the Actions summary without opening the full log.
set -o pipefail
"$@" 2>&1 | tee build.log
code=${PIPESTATUS[0]}
if [ "$code" -ne 0 ]; then
  awk '
    function flush() { if (buf != "") { gsub(/%/, "%25", buf); gsub(/\r/, "", buf); print "::error title=cargo::" buf; n++ } buf = "" }
    /^(error|warning: unused)/ { flush(); if (n >= 9) exit; buf = $0; next }
    buf != "" { if ($0 == "") { flush(); next } buf = buf "%0A" $0 }
    END { flush() }
  ' build.log
fi
exit "$code"
