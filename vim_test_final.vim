" Test 1: X command
enew
normal! ihello
normal! $X.
write! /tmp/vim_test_X.txt
quit!
