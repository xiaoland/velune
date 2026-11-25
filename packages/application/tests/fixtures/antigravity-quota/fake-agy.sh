#!/bin/sh
set -eu

fixture=${0%/*}
printf '%s\n' "$1" >> "$fixture/calls"
printf '%s\n' "$PWD" >> "$fixture/workspaces"

test "$AGY_CLI_DISABLE_AUTO_UPDATE" = true
test -z "${AGY_ADC_AUTH+x}${CLOUD_CODE_URL+x}${GEMINI_API_KEY+x}${GOOGLE_APPLICATION_CREDENTIALS+x}"
test "$(command -v open)" = "$BROWSER"
test "$(command -v xdg-open)" = "${BROWSER%/*}/xdg-open"
if "$BROWSER"; then
    exit 30
fi
if test -t 0 || IFS= read -r unexpected; then
    exit 31
fi
test -f .gemini/antigravity-cli/settings.json
test -f .agents/agents/velune-usage/agent.md

case "$1" in
    --version)
        shift
        test "$1" = --gemini-dir && test "$3" = --log-file || exit 32
        cat "$fixture/version"
        ;;
    -p)
        shift
        test "$1" = /usage && test "$2" = --output-format && test "$3" = json || exit 33
        test "$4" = --agent && test "$5" = velune-usage || exit 34
        test "$6" = --print-timeout && test "$7" = 12s || exit 35
        test "$8" = --gemini-dir && test "${10}" = --log-file || exit 36
        cat "$fixture/report"
        ;;
    *)
        exit 37
        ;;
esac
