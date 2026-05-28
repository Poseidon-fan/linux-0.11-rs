# Filesystem ops on /tmp (each test starts with a fresh disk image).

> mkdir /tmp/d
> ls -d /tmp/d
< /tmp/d

> echo content > /tmp/d/file
> cat /tmp/d/file
< content

> cp /tmp/d/file /tmp/d/copy
> cmp /tmp/d/file /tmp/d/copy
> echo $?
< 0

> rm /tmp/d/file /tmp/d/copy
> rmdir /tmp/d
> ls /tmp/d
~ (?i)no such|not found|cannot|nonexist
