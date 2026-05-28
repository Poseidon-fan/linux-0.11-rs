# Basic shell smoke test: echo, exit status, arithmetic.

> echo hello world
< hello world

> true
> echo $?
< 0

> false
> echo $?
< 1

> echo $((1 + 2 * 3))
< 7
