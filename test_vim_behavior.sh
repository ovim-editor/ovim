#!/bin/bash
# Test actual Vim behavior for dot-repeat commands

echo "=== Testing X command with dot repeat ==="
echo -e "Input: hello"
echo -e "Keys: \$X."
echo "Expected: 'hel' (delete 'o', then 'l')"
nvim -u NONE -c 'normal! ihello' -c 'normal! $X.' -c 'w /tmp/test_X.txt' -c 'q!' /dev/null 2>/dev/null
echo -n "Actual: '"
cat /tmp/test_X.txt | tr -d '\n'
echo "'"
echo

echo "=== Testing o command with dot repeat ==="
echo -e "Input: line 1\\nline 2"
echo -e "Keys: onew<Esc>j."
echo "Expected: new line below line 1, then new line below line 2"
nvim -u NONE -c 'normal! iline 1' -c 'normal! oline 2' -c 'normal! ggonew' -c 'normal! j.' -c 'w /tmp/test_o.txt' -c 'q!' /dev/null 2>/dev/null
echo "Actual:"
cat /tmp/test_o.txt
echo

echo "=== Testing O command with dot repeat ==="
echo -e "Input: line 1\\nline 2"
echo -e "Keys: Onew<Esc>j."
nvim -u NONE -c 'normal! iline 1' -c 'normal! oline 2' -c 'normal! ggOnew' -c 'normal! j.' -c 'w /tmp/test_O.txt' -c 'q!' /dev/null 2>/dev/null
echo "Actual:"
cat /tmp/test_O.txt
echo

echo "=== Testing r command with dot repeat ==="
echo -e "Input: hello world"
echo -e "Keys: rXl."
nvim -u NONE -c 'normal! ihello world' -c 'normal! 0rXl.' -c 'w /tmp/test_r.txt' -c 'q!' /dev/null 2>/dev/null
echo "Actual:"
cat /tmp/test_r.txt
echo

echo "=== Testing R command with dot repeat ==="
echo -e "Input: hello world"
echo -e "Keys: RXXX<Esc>w."
nvim -u NONE -c 'normal! ihello world' -c 'normal! 0RXXX' -c 'normal! w.' -c 'w /tmp/test_R.txt' -c 'q!' /dev/null 2>/dev/null
echo "Actual:"
cat /tmp/test_R.txt
echo

echo "=== Testing ci\" with dot repeat ==="
echo -e 'Input: "hello" "world"'
echo -e 'Keys: ci"new<Esc>f".'
nvim -u NONE -c 'normal! i"hello" "world"' -c 'normal! 0f"ci"new' -c 'normal! f".' -c 'w /tmp/test_ciq.txt' -c 'q!' /dev/null 2>/dev/null
echo "Actual:"
cat /tmp/test_ciq.txt
echo

echo "=== Testing di( with dot repeat ==="
echo -e 'Input: (foo) (bar)'
echo -e 'Keys: di(f(.'
nvim -u NONE -c 'normal! i(foo) (bar)' -c 'normal! 0f(di(' -c 'normal! f(.' -c 'w /tmp/test_dip.txt' -c 'q!' /dev/null 2>/dev/null
echo "Actual:"
cat /tmp/test_dip.txt
echo

echo "=== Testing cw with dot repeat at different word lengths ==="
echo -e 'Input: short longerword'
echo -e 'Keys: cwX<Esc>w.'
nvim -u NONE -c 'normal! ishort longerword' -c 'normal! 0cwX' -c 'normal! w.' -c 'w /tmp/test_cw.txt' -c 'q!' /dev/null 2>/dev/null
echo "Actual:"
cat /tmp/test_cw.txt
echo
