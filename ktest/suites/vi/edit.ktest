# vi smoke test: open a file, insert text, save, verify on disk.
#
# `vi` takes over the screen so a normal `>` (which waits for the shell
# prompt) would hang — send the launch line raw and only re-sync with
# the prompt after `:wq` exits.

> rm -f /tmp/v.txt
! send "vi /tmp/v.txt\r"
! sleep 0.5
! send "ihello vi\e:wq\r"
! wait-prompt

> cat /tmp/v.txt
< hello vi
