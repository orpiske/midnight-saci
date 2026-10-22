#!/bin/sh
# Print a greeting. Changes nothing. Shows that a value with shell
# characters, for example "$(id)", arrives as one literal argument.
#
#   greet.sh --name NAME [--language en|pt]
set -eu
name=""
language=en
while [ $# -gt 0 ]; do
    case "$1" in
        --name) name=$2; shift 2 ;;
        --language) language=$2; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done
case "$language" in
    pt) printf 'Olá, %s!\n' "$name" ;;
    *) printf 'Hello, %s!\n' "$name" ;;
esac
