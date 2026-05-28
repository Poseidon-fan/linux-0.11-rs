# Filesystem ops: ls, mkdir, rm, rmdir, cp, mv, ln/link, touch, cat redirect.
#
# All scratch state lives under /tmp/kt-fs/. The first step wipes it
# so we can re-enter the test cleanly.

> rm -rf /tmp/kt-fs && mkdir /tmp/kt-fs

# --- mkdir / rmdir -----------------------------------------------------

> mkdir /tmp/kt-fs/d1
> ls -d /tmp/kt-fs/d1
< /tmp/kt-fs/d1

> mkdir -p /tmp/kt-fs/d2/d3/d4
> ls -d /tmp/kt-fs/d2/d3/d4
< /tmp/kt-fs/d2/d3/d4

> rmdir /tmp/kt-fs/d2/d3/d4
> ls /tmp/kt-fs/d2/d3
~ ^\s*$|total 0

# --- touch -------------------------------------------------------------

> touch /tmp/kt-fs/t1 /tmp/kt-fs/t2
> ls /tmp/kt-fs/t1 /tmp/kt-fs/t2
< /tmp/kt-fs/t1
< /tmp/kt-fs/t2

# --- cat + write redirection ------------------------------------------

> echo hello > /tmp/kt-fs/h
> cat /tmp/kt-fs/h
< hello

> printf 'line1\nline2\n' > /tmp/kt-fs/lines
> cat /tmp/kt-fs/lines
< line1
< line2

# --- append redirection -----------------------------------------------

> echo first > /tmp/kt-fs/app
> echo second >> /tmp/kt-fs/app
> cat /tmp/kt-fs/app
< first
< second

# --- cp ----------------------------------------------------------------

> cp /tmp/kt-fs/h /tmp/kt-fs/h.copy
> cat /tmp/kt-fs/h.copy
< hello
> cmp /tmp/kt-fs/h /tmp/kt-fs/h.copy
> echo cp_cmp=$?
< cp_cmp=0

# --- mv ----------------------------------------------------------------

> echo moveme > /tmp/kt-fs/src
> mv /tmp/kt-fs/src /tmp/kt-fs/dst
> cat /tmp/kt-fs/dst
< moveme
> ls /tmp/kt-fs/src
~ (?i)no such|not found|cannot

# --- ln / link ---------------------------------------------------------

> echo linked > /tmp/kt-fs/target
> link /tmp/kt-fs/target /tmp/kt-fs/hardlink
> cat /tmp/kt-fs/hardlink
< linked

# --- rm ----------------------------------------------------------------

> touch /tmp/kt-fs/del
> rm /tmp/kt-fs/del
> ls /tmp/kt-fs/del
~ (?i)no such|not found|cannot

> mkdir -p /tmp/kt-fs/rmrf/inner
> touch /tmp/kt-fs/rmrf/inner/x
> rm -r /tmp/kt-fs/rmrf
> ls /tmp/kt-fs/rmrf
~ (?i)no such|not found|cannot

# --- ls flags ----------------------------------------------------------

> ls /tmp/kt-fs | head -n 1
~ \S

> ls -l /tmp/kt-fs/h
~ \S+\s+\S+\s+\S+\s+\S+\s+\d+\s+.*h

> ls -a /tmp/kt-fs | head -n 2
~ \.\.?

# --- pwd / cd ----------------------------------------------------------

> cd /tmp/kt-fs
> pwd
< /tmp/kt-fs

> cd /
> pwd
< /

# --- diff / cmp --------------------------------------------------------

> printf 'a\nb\n' > /tmp/kt-fs/dA
> printf 'a\nb\n' > /tmp/kt-fs/dB
> cmp /tmp/kt-fs/dA /tmp/kt-fs/dB
> echo cmp_same=$?
< cmp_same=0

> printf 'a\nb\n' > /tmp/kt-fs/dC
> printf 'a\nX\n' > /tmp/kt-fs/dD
> cmp /tmp/kt-fs/dC /tmp/kt-fs/dD
> echo cmp_diff=$?
~ cmp_diff=[12]

> diff /tmp/kt-fs/dA /tmp/kt-fs/dB
> echo diff_same=$?
< diff_same=0

> diff /tmp/kt-fs/dC /tmp/kt-fs/dD
> echo diff_diff=$?
~ diff_diff=[12]

# --- stat / du / df ---------------------------------------------------

> echo abc > /tmp/kt-fs/sized
> stat /tmp/kt-fs/sized
~ (?i)size|file

> du /tmp/kt-fs/sized
~ ^\s*\d+

> df /tmp
~ (?i)filesystem|mounted|/

# --- cleanup -----------------------------------------------------------

> cd /
> rm -rf /tmp/kt-fs
