#!/usr/bin/env bash
# Verify specific edge cases more carefully

set -euo pipefail

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ovim-edge-cases.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
TEST_FILE="$TMP_DIR/test_input.txt"

echo "=== Test 1: X command (delete before cursor) ==="
echo "Input: 'hello'"
echo "Commands: \$ (end) → X (delete 'o') → . (repeat, delete 'l')"
printf "hello" > "$TEST_FILE"
nvim -u NONE -n -es "$TEST_FILE" <<EOF
normal! \$X.
wq
EOF
echo -n "Result: '"
cat "$TEST_FILE"
echo "'"
echo "Expected: 'hel'"
echo

echo "=== Test 2: o command (open below) ==="
printf "line 1\nline 2" > "$TEST_FILE"
nvim -u NONE -n -es "$TEST_FILE" <<EOF
normal! ggo
normal! inew
normal! \x1b
normal! j.
wq
EOF
echo "Result:"
cat "$TEST_FILE"
echo "Expected: line 1, new, line 2, new"
echo

echo "=== Test 3: O command (open above) ==="
printf "line 1\nline 2" > "$TEST_FILE"
nvim -u NONE -n -es "$TEST_FILE" <<EOF
normal! ggO
normal! inew
normal! \x1b
normal! j.
wq
EOF
echo "Result:"
cat "$TEST_FILE"
echo "Expected: new, line 1, new, line 2"
echo

echo "=== Test 4: r command (replace char) ==="
printf "hello world" > "$TEST_FILE"
nvim -u NONE -n -es "$TEST_FILE" <<EOF
normal! 0rXl.
wq
EOF
echo -n "Result: '"
cat "$TEST_FILE"
echo "'"
echo "Expected: 'XXllo world'"
echo

echo "=== Test 5: R command (replace mode) ==="
printf "hello world" > "$TEST_FILE"
nvim -u NONE -n -es "$TEST_FILE" <<EOF
normal! 0R
normal! iXXX
normal! \x1b
normal! w.
wq
EOF
echo -n "Result: '"
cat "$TEST_FILE"
echo "'"
echo "Expected: 'XXXlo XXXld' (both words start with XXX)"
echo

echo "=== Test 6: ci\" (change inside quotes) ==="
printf '"hello" "world"' > "$TEST_FILE"
nvim -u NONE -n -es "$TEST_FILE" <<EOF
normal! 0f"ci"
normal! inew
normal! \x1b
normal! f".
wq
EOF
echo -n "Result: '"
cat "$TEST_FILE"
echo "'"
echo "Expected: '\"new\" \"new\"'"
echo

echo "=== Test 7: di( (delete inside parens) ==="
printf "(foo) (bar)" > "$TEST_FILE"
nvim -u NONE -n -es "$TEST_FILE" <<EOF
normal! 0f(di(f(.
wq
EOF
echo -n "Result: '"
cat "$TEST_FILE"
echo "'"
echo "Expected: '() ()'"
echo

echo "=== Test 8: cw (change word) ==="
printf "short longerword" > "$TEST_FILE"
nvim -u NONE -n -es "$TEST_FILE" <<EOF
normal! 0cw
normal! iX
normal! \x1b
normal! w.
wq
EOF
echo -n "Result: '"
cat "$TEST_FILE"
echo "'"
echo "Expected: 'X X'"
echo
