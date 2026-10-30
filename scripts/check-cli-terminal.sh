#!/usr/bin/env bash
# Check real terminals with Bash 3.2+, BSD/macOS or GNU/Linux script, sqlite3,
# jq, stty, and standard Unix tools. Build stm before running this script.
# Usage: bash scripts/check-cli-terminal.sh [path/to/stm]

set -euo pipefail

# Record stm's exit code without relying on platform-specific script behavior.
terminal_child() {
    local stream=$1 capture=$2 binary=$3 root=$4 code
    shift 4
    exec 9<&-
    stty -echo rows 40 cols "${STM_TEST_COLUMNS:-80}"
    if [ "$stream" = stdout ]; then
        "$binary" --data-dir "$root" --offline "$@" </dev/null 2>"$capture/stderr" &
    elif [ "$stream" = both ]; then
        "$binary" --data-dir "$root" --offline "$@" </dev/null &
    else
        "$binary" --data-dir "$root" --offline "$@" </dev/null >"$capture/stdout" &
    fi
    local child=$!
    printf '%s\n' "$child" >"$capture/child.pid"
    if wait "$child"; then code=0; else code=$?; fi
    printf '%s\n' "$code" >"$capture/exit"
}

if [ "${1:-}" = --terminal-child ]; then
    shift
    terminal_child "$@"
    exit 0
fi

# Report the active case and its three separate output captures.
fail() {
    printf 'error: %s: %s\n' "${label:-terminal checker}" "$*" >&2
    if [ -n "${capture:-}" ]; then
        local file
        for file in terminal stdout stderr driver-stderr; do
            if [ -s "$capture/$file" ]; then
                printf '%s:\n' "$file" >&2
                cat "$capture/$file" >&2
            fi
        done
    fi
    exit 1
}

# End the input holder before it can leave a background sleep behind.
stop_input() {
    if [ -n "$input_pid" ]; then
        kill "$input_pid" 2>/dev/null || : # The holder may already have exited.
        wait "$input_pid" 2>/dev/null || :
        input_pid=''
        exec 9<&-
    fi
}

# Stop only this check's recorded processes; either may already have exited.
stop_terminal() {
    if [ -n "$driver_pid" ]; then
        local child=''
        if [ -s "$capture/child.pid" ]; then read -r child <"$capture/child.pid"; fi
        case "$child" in
            ''|*[!0-9]*) ;;
            *) kill -TERM "$child" 2>/dev/null || : ;;
        esac
        kill -TERM "$driver_pid" 2>/dev/null || :
        sleep 1
        case "$child" in
            ''|*[!0-9]*) ;;
            *) kill -KILL "$child" 2>/dev/null || : ;;
        esac
        kill -KILL "$driver_pid" 2>/dev/null || :
        wait "$driver_pid" 2>/dev/null || :
        driver_pid=''
    fi
    stop_input
}

# Remove only the temporary directory created by this checker.
cleanup() {
    stop_terminal
    if [ -n "$work" ]; then rm -rf "$work"; fi
}

# Capture one PTY stream and redirect the other, with a 15-second deadline.
run_terminal() {
    local stream=$1 expected_code=$2 started=$SECONDS driver_code code command
    shift 2
    capture=$work/capture
    mkdir -p "$capture"
    : >"$capture/terminal"
    : >"$capture/stdout"
    : >"$capture/stderr"
    : >"$capture/driver-stderr"
    rm -f "$capture/exit" "$capture/child.pid"
    # A live pipe prevents BSD script from echoing its synthetic EOF into JSON.
    exec 9< <(sleep 30)
    input_pid=$!
    case "$platform" in
        Darwin)
            script -q /dev/null "$BASH" "$self" --terminal-child "$stream" \
                "$capture" "$binary" "$root" "$@" <&9 \
                >"$capture/terminal" 2>"$capture/driver-stderr" &
            ;;
        Linux)
            printf -v command '%q ' "$BASH" "$self" --terminal-child "$stream" \
                "$capture" "$binary" "$root" "$@"
            SHELL=$BASH script -q -c "$command" /dev/null <&9 \
                >"$capture/terminal" 2>"$capture/driver-stderr" &
            ;;
    esac
    driver_pid=$!
    while kill -0 "$driver_pid" 2>/dev/null; do
        if [ $((SECONDS - started)) -ge 15 ]; then
            stop_terminal
            fail "command timed out: $*"
        fi
        sleep 0.05
    done
    if wait "$driver_pid"; then driver_code=0; else driver_code=$?; fi
    driver_pid=''
    stop_input
    [ "$driver_code" -eq 0 ] || fail "script exited $driver_code"
    [ ! -s "$capture/driver-stderr" ] || fail 'script wrote an error'
    [ -s "$capture/exit" ] || fail 'the command did not record its exit code'
    read -r code <"$capture/exit"
    [ "$code" = "$expected_code" ] || fail "expected exit $expected_code, got $code: $*"
    checks=$((checks + 1))
}

# Require the real command's selected accent and the expected stream split.
check_accent() {
    local stream=$1 code=$2 expected=$3
    shift 3
    run_terminal "$stream" "$code" "$@"
    grep -Fq "$expected" "$capture/terminal" || fail "missing accent $expected"
    [ ! -s "$capture/stdout" ] && [ ! -s "$capture/stderr" ] || fail 'output used the wrong stream'
    if [ "$expected" = '35m' ]; then
        if grep -Fq '38;2;' "$capture/terminal"; then fail 'ANSI fallback used RGB'; fi
    fi
    if [ "$stream" = stderr ]; then
        grep -Fq '31m' "$capture/terminal" || fail 'error lost its red status color'
    fi
}

# Reject escapes on all command streams in plain modes.
assert_plain() {
    local file
    for file in terminal stdout stderr; do
        if LC_ALL=C grep -Fq "$escape" "$capture/$file"; then fail "escape in plain $file"; fi
    done
}

# Check real command wiring for both RGB accents and both ANSI fallback paths.
check_accents() {
    local scenario expected
    for scenario in dark-rgb light-rgb unknown-ansi no-rgb; do
        case "$scenario" in
            dark-rgb)
                export COLORFGBG='15;0' COLORTERM=truecolor
                expected='38;2;255;92;154m' ;;
            light-rgb)
                export COLORFGBG='0;15' COLORTERM=24bit
                expected='38;2;151;23;77m' ;;
            unknown-ansi)
                export COLORFGBG='0;unknown' COLORTERM=truecolor
                expected='35m' ;;
            no-rgb)
                export COLORFGBG='0;15' COLORTERM=''
                expected='35m' ;;
        esac
        label=$scenario
        check_accent stdout 0 "$expected" --help
        check_accent stdout 0 "$expected" collection query --help
        check_accent stdout 0 "$expected" card 'Contrast card'
        check_accent stderr 3 "$expected" card Missing
        check_accent stderr 2 "$expected" query --max-price bad
    done
}

# Keep plain modes and independently redirected streams unchanged.
check_plain_streams() {
    local background override
    local -a prefix
    for background in '15;0' '0;15' '0;8'; do
        export COLORFGBG=$background COLORTERM=truecolor
        for override in NO_COLOR dumb flag; do
            unset NO_COLOR
            export TERM=xterm-256color
            prefix=()
            case "$override" in
                NO_COLOR) export NO_COLOR=1 ;;
                dumb) export TERM=dumb ;;
                flag) prefix=(--no-color) ;;
            esac
            label="plain background=$background override=$override"
            run_terminal stdout 0 ${prefix[@]+"${prefix[@]}"} --help
            assert_plain
            run_terminal stdout 0 ${prefix[@]+"${prefix[@]}"} card 'Contrast card'
            assert_plain
            run_terminal stderr 2 ${prefix[@]+"${prefix[@]}"} query --max-price bad
            assert_plain
        done
        unset NO_COLOR
        export TERM=xterm-256color
        label="JSON and redirection background=$background"
        run_terminal stderr 0 query --json --help
        assert_plain
        run_terminal stderr 3 card Missing --json
        assert_plain
        run_terminal stdout 0 card 'Contrast card' --json
        assert_plain
        jq -e '.name == "Contrast card"' "$capture/terminal" >/dev/null || fail 'invalid terminal JSON'
        run_terminal stderr 0 --help
        assert_plain
        [ -s "$capture/stdout" ] && [ ! -s "$capture/terminal" ] && [ ! -s "$capture/stderr" ] \
            || fail 'redirected help used the wrong stream'
        run_terminal stdout 2 query --max-price bad
        assert_plain
        [ -s "$capture/stderr" ] && [ ! -s "$capture/terminal" ] && [ ! -s "$capture/stdout" ] \
            || fail 'redirected Clap error used the wrong stream'
    done
}

# Keep full card names on narrow screens and skip prompts with redirected input.
check_narrow_and_redirected_input() {
    label='narrow Unicode card'
    export STM_TEST_COLUMNS=32
    run_terminal stdout 0 --no-color card 'Jötun 漢字 é'
    assert_plain
    grep -Fq 'Jötun 漢字 é' "$capture/terminal" || fail 'narrow output lost the card name'
    unset STM_TEST_COLUMNS

    label='terminal output with redirected stdin'
    run_terminal both 0 --no-color deck import 'Redirected input' "$root/import.txt"
    assert_plain
    grep -Fq 'no commander section; candidates: Candidate commander' "$capture/terminal" \
        || fail 'redirected stdin did not use the noninteractive commander note'
    if grep -Fq 'Choose a commander' "$capture/terminal"; then
        fail 'redirected stdin opened a commander prompt'
    fi
}

# Seed an isolated store with the current schema and status fields.
main() {
    [ "$#" -le 1 ] || fail 'usage: bash scripts/check-cli-terminal.sh [path/to/stm]'
    local tool
    for tool in script sqlite3 jq stty mktemp grep sleep uname; do
        command -v "$tool" >/dev/null || fail "required command not found: $tool"
    done
    platform=$(uname -s)
    case "$platform" in Darwin|Linux) ;; *) fail 'requires BSD/macOS or GNU/Linux script' ;; esac
    self=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")
    binary=${1:-target/debug/stm}
    [ -x "$binary" ] || fail "build stm first; executable not found: $binary"
    binary=$(cd "$(dirname "$binary")" && pwd)/$(basename "$binary")
    unset NO_COLOR COLORFGBG CLICOLOR CLICOLOR_FORCE FORCE_COLOR
    export TERM=xterm-256color
    work=$(mktemp -d "${TMPDIR:-/tmp}/stm-cli-terminal.XXXXXX")
    root=$work/store
    label='seed store'
    run_terminal stdout 3 query --json
    assert_plain
    jq -e '. == []' "$capture/terminal" >/dev/null || fail 'invalid empty-store JSON'
    [ ! -s "$capture/stderr" ] || fail 'seed command wrote an error'
    sqlite3 "$root/stm.db" <<'SQL'
INSERT INTO cards (name, oracle_id, mana_cost, type_line)
VALUES ('Contrast card', 'contrast', '{W}{U}{B}{R}{G}{C}', 'Instant');
INSERT INTO cards (name, oracle_id, type_line)
VALUES ('Jötun 漢字 é', 'unicode', 'Instant'),
       ('Candidate commander', 'candidate', 'Legendary Creature — Test');
SQL
    cat >"$root/import.txt" <<'DECK'
// DECK
1 Contrast card
// SIDEBOARD
1 Candidate commander
DECK
    cat >"$root/status.json" <<'JSON'
{
  "setup_complete": true,
  "ingested_cards": 1,
  "embedded_cards": 0,
  "model": "",
  "dim": 0,
  "names": [],
  "scryfall_synced_at": "",
  "doc_version": 0,
  "combos_synced_at": ""
}
JSON
    check_accents
    check_plain_streams
    check_narrow_and_redirected_input
    printf 'Passed %s terminal checks.\n' "$checks"
}

work='' capture='' driver_pid='' input_pid='' checks=0
escape=$'\033'
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
main "$@"
