# Coreutils sanity: a handful of standard tools agree with their docs.

> echo abc | wc -c
~ ^\s*4\b

> echo hello | tr a-z A-Z
< HELLO

> seq 1 3
< 1
< 2
< 3

> printf 'a\nb\nc\n' | head -n 2
< a
< b

> printf 'x\nx\ny\n' | uniq -c
~ \s*2\s+x
~ \s*1\s+y
