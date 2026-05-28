# Search / match / numeric: grep, find, expr, seq, true, false, test.

> rm -rf /tmp/kt-search && mkdir /tmp/kt-search

# --- true / false ------------------------------------------------------

> true
> echo true_rc=$?
< true_rc=0

> false
> echo false_rc=$?
< false_rc=1

# --- test / [ ] --------------------------------------------------------

> [ 1 -eq 1 ]
> echo eq_rc=$?
< eq_rc=0

> [ 1 -eq 2 ]
> echo ne_rc=$?
< ne_rc=1

> [ -z "" ]
> echo z_rc=$?
< z_rc=0

> [ -n abc ]
> echo n_rc=$?
< n_rc=0

> [ "a" = "a" ]
> echo eq_str_rc=$?
< eq_str_rc=0

> touch /tmp/kt-search/exists
> [ -f /tmp/kt-search/exists ]
> echo file_rc=$?
< file_rc=0

> [ -d /tmp/kt-search ]
> echo dir_rc=$?
< dir_rc=0

> [ -f /tmp/kt-search/doesnotexist ]
> echo nfile_rc=$?
< nfile_rc=1

# --- expr --------------------------------------------------------------

> expr 2 + 3
< 5

> expr 7 \* 6
< 42

> expr 10 / 3
< 3

> expr 10 % 3
< 1

> expr length abcd
< 4

> expr 1 = 1
< 1

> expr 1 = 2
< 0

# --- seq ---------------------------------------------------------------

> seq 1 3
< 1
< 2
< 3

> seq 3
< 1
< 2
< 3

> seq 1 2 5
< 1
< 3
< 5

# --- grep --------------------------------------------------------------

> printf 'apple\nbanana\napricot\n' > /tmp/kt-search/fruit
> grep ap /tmp/kt-search/fruit
< apple
< apricot

> grep -v ap /tmp/kt-search/fruit
< banana

> grep -c ap /tmp/kt-search/fruit
< 2

> grep -n ap /tmp/kt-search/fruit
~ 1:apple
~ 3:apricot

> printf 'foo\nbar\n' | grep foo
< foo

# --- find --------------------------------------------------------------

> mkdir -p /tmp/kt-search/sub
> touch /tmp/kt-search/sub/leaf
> find /tmp/kt-search
~ /tmp/kt-search$
~ /tmp/kt-search/sub
~ /tmp/kt-search/sub/leaf

> find /tmp/kt-search -name leaf
~ /tmp/kt-search/sub/leaf

# --- which -------------------------------------------------------------

> which echo
~ /bin/echo|echo

# --- cleanup -----------------------------------------------------------

> rm -rf /tmp/kt-search
