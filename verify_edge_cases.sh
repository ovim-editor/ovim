#!/bin/bash
# Verify specific edge cases more carefully

echo "=== Test 1: X command (delete before cursor) ==="
echo "Input: 'hello'"
echo "Commands: \$ (end) → X (delete 'o') → . (repeat, delete 'l')"
printf "hello" > /tmp/test_input.txt
nvim -u NONE -n -es /tmp/test_input.txt <<EOF
normal! \$X.
wq
EOF
echo -n "Result: '"
cat /tmp/test_input.txt
echo "'"
echo "Expected: 'hel'"
echo

echo "=== Test 2: o command (open below) ==="
printf "line 1\nline 2" > /tmp/test_input.txt
nvim -u NONE -n -es /tmp/test_input.txt <<EOF
normal! ggo
normal! inew
normal! \x1b
normal! j.
wq
EOF
echo "Result:"
cat /tmp/test_input.txt
echo "Expected: line 1, new, line 2, new"
echo

echo "=== Test 3: O command (open above) ==="
printf "line 1\nline 2" > /tmp/test_input.txt
nvim -u NONE -n -es /tmp/test_input.txt <<EOF
normal! ggO
normal! inew
normal! \x1b
normal! j.
wq
EOF
echo "Result:"
cat /tmp/test_input.txt
echo "Expected: new, line 1, new, line 2"
echo

echo "=== Test 4: r command (replace char) ==="
printf "hello world" > /tmp/test_input.txt
nvim -u NONE -n -es /tmp/test_input.txt <<EOF
normal! 0rXl.
wq
EOF
echo -n "Result: '"
cat /tmp/test_input.txt
echo "'"
echo "Expected: 'XXllo world'"
echo

echo "=== Test 5: R command (replace mode) ==="
printf "hello world" > /tmp/test_input.txt
nvim -u NONE -n -es /tmp/test_input.txt <<EOF
normal! 0R
normal! iXXX
normal! \x1b
normal! w.
wq
EOF
echo -n "Result: '"
cat /tmp/test_input.txt
echo "'"
echo "Expected: 'XXXlo XXXld' (both words start with XXX)"
echo

echo "=== Test 6: ci\" (change inside quotes) ==="
printf '"hello" "world"' > /tmp/test_input.txt
nvim -u NONE -n -es /tmp/test_input.txt <<EOF
normal! 0f"ci"
normal! inew
normal! \x1b
normal! f".
wq
EOF
echo -n "Result: '"
cat /tmp/test_input.txt
echo "'"
echo "Expected: '\"new\" \"new\"'"
echo

echo "=== Test 7: di( (delete inside parens) ==="
printf "(foo) (bar)" > /tmp/test_input.txt
nvim -u NONE -n -es /tmp/test_input.txt <<EOF
normal! 0f(di(f(.
wq
EOF
echo -n "Result: '"
cat /tmp/test_input.txt
echo "'"
echo "Expected: '() ()'"
echo

echo "=== Test 8: cw (change word) ==="
printf "short longerword" > /tmp/test_input.txt
nvim -u NONE -n -es /tmp/test_input.txt <<EOF
normal! 0cw
normal! iX
normal! \x1b
normal! w.
wq
EOF
echo -n "Result: '"
cat /tmp/test_input.txt
echo "'"
echo "Expected: 'X X'"
echo
