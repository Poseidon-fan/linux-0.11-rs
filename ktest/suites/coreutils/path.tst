# Path manipulation + multi-file utilities: basename, dirname,
# comm, join, split.

> rm -rf /tmp/kt-path && mkdir /tmp/kt-path

# --- basename ----------------------------------------------------------

> basename /a/b/c.txt
< c.txt

> basename /a/b/c.txt .txt
< c

> basename plain
< plain

> basename /
< /

# --- dirname -----------------------------------------------------------

> dirname /a/b/c.txt
< /a/b

> dirname plain
< .

> dirname /
< /

# --- comm --------------------------------------------------------------

> printf 'a\nb\n' > /tmp/kt-path/c1
> printf 'a\nc\n' > /tmp/kt-path/c2
> comm /tmp/kt-path/c1 /tmp/kt-path/c2
~ b
~ c
~ a

> comm -12 /tmp/kt-path/c1 /tmp/kt-path/c2
< a

> comm -23 /tmp/kt-path/c1 /tmp/kt-path/c2
< b

# --- join --------------------------------------------------------------

> printf 'a 1\nb 2\n' > /tmp/kt-path/j1
> printf 'a x\nb y\n' > /tmp/kt-path/j2
> join /tmp/kt-path/j1 /tmp/kt-path/j2
< a 1 x
< b 2 y

# --- split -------------------------------------------------------------

> printf 'a\nb\nc\nd\n' > /tmp/kt-path/sp
> cd /tmp/kt-path
> split -l 2 sp PART_
> ls PART_aa PART_ab
< PART_aa
< PART_ab
> cat PART_aa
< a
< b
> cat PART_ab
< c
< d
> cd /

# --- cleanup -----------------------------------------------------------

> rm -rf /tmp/kt-path
