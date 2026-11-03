#!/usr/bin/env bash
# Check the docs/ tree against the page conventions in
# docs/contributing/documentation.md. Works with bash 3.2 and the BSD and
# GNU tools, so it runs on Linux and macOS. Run it from the repository
# root: .github/check-docs.sh
set -u

DOCS=docs
errors=0

fail() {
    printf '%s\n' "$1"
    errors=$((errors + 1))
}

# Prints the file without fenced code blocks and inline code spans.
prose() {
    awk '
        /^[ \t]*(```|~~~)/ { fenced = !fenced; next }
        !fenced { gsub(/`[^`]*`/, ""); print }
    ' "$1"
}

# Prints the headings of the file, outside fenced code blocks.
headings() {
    awk '
        /^[ \t]*(```|~~~)/ { fenced = !fenced; next }
        !fenced && /^#+ / { print }
    ' "$1"
}

# Old paths in redirects.txt. A file at an old path is a stub.
redirects="$DOCS/redirects.txt"
if [ ! -f "$redirects" ]; then
    fail "$redirects: missing"
    exit 1
fi
stubs=$(awk '!/^#/ && NF { print $1 }' "$redirects")

is_stub() {
    printf '%s\n' "$stubs" | grep -qxF "$1"
}

lineno=0
while IFS= read -r line; do
    lineno=$((lineno + 1))
    case "$line" in '#'* | '') continue ;; esac
    set -- $line
    if [ "$#" -ne 2 ]; then
        fail "$redirects:$lineno: expected 'old-path new-path'"
        continue
    fi
    [ -f "$DOCS/$2" ] || fail "$redirects:$lineno: target $2 has no page"
    [ "$1" != "$2" ] || fail "$redirects:$lineno: $1 redirects to itself"
done <"$redirects"

pages=0
for page in $(find "$DOCS" -name '*.md' -not -path "$DOCS/templates/*" | sort); do
    rel=${page#"$DOCS"/}
    dir=$(dirname "$page")
    name=$(basename "$page")
    pages=$((pages + 1))

    # Relative links must point at an existing file or directory.
    for target in $(prose "$page" | grep -oE '\]\([^)[:space:]]+\)' | sed -e 's/^](//' -e 's/)$//'); do
        case "$target" in
            *://* | mailto:* | '#'*) continue ;;
        esac
        path=${target%%#*}
        [ -e "$dir/$path" ] || fail "$page: link $target has no target"
    done

    if is_stub "$rel"; then
        continue
    fi

    # Design notes keep their original form; only their hub is checked.
    case "$rel" in
        contributing/design/*) [ "$name" = index.md ] || { [ -n "$(grep -F "]($name)" "$dir/index.md" 2>/dev/null)" ] || fail "$page: not listed in $dir/index.md"; continue; } ;;
    esac

    heads=$(headings "$page")
    h1s=$(printf '%s\n' "$heads" | grep -c '^# ')
    [ "$h1s" -eq 1 ] || fail "$page: has $h1s H1 headings, expected 1"
    head -n 1 "$page" | grep -q '^# ' || fail "$page: does not start with the H1"
    last_h2=$(printf '%s\n' "$heads" | grep '^## ' | tail -n 1)
    [ "$last_h2" = "## Next steps" ] || fail "$page: the last section must be '## Next steps'"
    if printf '%s\n' "$heads" | grep -q '^#### '; then
        fail "$page: has a heading below H3"
    fi
    if prose "$page" | grep -qE '^> \*\*(Note|Tip|Important|Warning):?\*\*'; then
        fail "$page: has a bold callout, use a GitHub alert such as '> [!NOTE]'"
    fi
    if grep -q "$(printf '\342\200\224')" "$page"; then
        fail "$page: has an em-dash"
    fi

    # Every page except the docs home is listed in the hub of its folder.
    if [ "$page" != "$DOCS/index.md" ]; then
        if [ "$name" = index.md ]; then
            parent=$(dirname "$dir")
            [ -n "$(grep -F "]($(basename "$dir")/index.md)" "$parent/index.md" 2>/dev/null)" ] ||
                fail "$page: not listed in $parent/index.md"
        else
            [ -f "$dir/index.md" ] || fail "$dir: has no index.md hub"
            [ -n "$(grep -F "]($name)" "$dir/index.md" 2>/dev/null)" ] || fail "$page: not listed in $dir/index.md"
        fi
    fi
done

if [ "$errors" -gt 0 ]; then
    printf '%d problem(s)\n' "$errors"
    exit 1
fi
printf 'OK: %d pages\n' "$pages"
