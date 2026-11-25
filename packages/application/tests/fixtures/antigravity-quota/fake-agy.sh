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
test -f .gemini/antigravity-cli/settings.json
test -f .agents/agents/velune-usage/agent.md

test ! -t 0
if test "$1" != acp && IFS= read -r unexpected; then
    exit 31
fi
case "$1" in
    --version)
        shift
        test "$1" = --gemini-dir && test "$3" = --log-file || exit 32
        cat "$fixture/version"
        ;;
    acp)
        shift
        test "$1" = --process-per-session && test "$2" = --sandbox || exit 38
        test "$3" = --disable-telemetry && test "$4" = --gemini-dir || exit 39
        IFS= read -r frame
        case "$frame" in *'"id":1'*'"method":"initialize"'*) ;; *) exit 40 ;; esac
        printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"sessionCapabilities":{"close":{}}}}}'
        IFS= read -r frame
        case "$frame" in *'"id":2'*'"method":"session/new"'*'"mcpServers":[]'*) ;; *) exit 41 ;; esac
        cat "$fixture/session"
        if IFS= read -r frame; then
            case "$frame" in *'"id":3'*'"method":"session/close"'*) ;; *) exit 42 ;; esac
            printf '%s\n' 'session/close' >> "$fixture/calls"
            printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{}}'
        fi
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
