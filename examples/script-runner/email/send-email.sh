#!/usr/bin/env bash
# Email the summary of a finished Saci run with mutt. The Saci backend calls
# this script through the script runner as an outcome action, so the script
# needs no access to the backend: all values come from the arguments.
#
#   send-email.sh --run-id ID --title TITLE --outcome OUTCOME \
#       --finished-at TIME --counts COUNTS [--reason TEXT] [--summary TEXT]
#
# Settings (the script runner passes only the variables in
# `execution.environment`):
#   SACI_EMAIL_TO  Recipient address. Required.
#   HOME           mutt reads ~/.muttrc (SMTP settings) from here.
#
# Requires mutt. Works on Linux and macOS.
set -euo pipefail

usage() {
    echo "usage: send-email.sh --run-id ID --title TITLE --outcome OUTCOME --finished-at TIME --counts COUNTS [--reason TEXT] [--summary TEXT]" >&2
    exit 2
}

run_id="" title="" outcome="" finished_at="" counts="" reason="" summary=""
while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || usage
    case "$1" in
        --run-id) run_id=$2 ;;
        --title) title=$2 ;;
        --outcome) outcome=$2 ;;
        --finished-at) finished_at=$2 ;;
        --counts) counts=$2 ;;
        --reason) reason=$2 ;;
        --summary) summary=$2 ;;
        *) usage ;;
    esac
    shift 2
done
[ -n "$run_id" ] && [ -n "$title" ] && [ -n "$outcome" ] && [ -n "$finished_at" ] || usage
: "${SACI_EMAIL_TO:?set SACI_EMAIL_TO to the recipient address}"

# Units run: the sum of the numbers in counts, for example "3 passed, 1 failed".
units=0
for word in $counts; do
    case "${word%,}" in
        ''|*[!0-9]*) ;;
        *) units=$((units + ${word%,})) ;;
    esac
done

subject="Saci Report: $title - $outcome - ${finished_at:0:10}"
body=$(cat <<BODY
Plan:         $title
Run:          $run_id
Outcome:      $outcome${reason:+ ($reason)}
Finished at:  $finished_at
Units run:    $units
Counts:       ${counts:-none}

${summary}
BODY
)

printf '%s\n' "$body" | mutt -s "$subject" -- "$SACI_EMAIL_TO"
echo "sent the summary of $run_id to $SACI_EMAIL_TO: $subject"
