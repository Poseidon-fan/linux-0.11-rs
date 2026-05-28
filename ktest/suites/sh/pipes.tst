# Pipes and redirection.

> echo abc > /tmp/p.txt
> cat /tmp/p.txt
< abc

> printf 'a\nb\nc\n' | wc -l
~ ^\s*3\b

> echo hello | tr a-z A-Z
< HELLO

> seq 1 3 | tail -n 1
< 3
