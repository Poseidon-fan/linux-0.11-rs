# Permission / ownership commands: chmod, chown, chgrp.
# Guest runs as root so all of these should succeed unconditionally.

> rm -rf /tmp/kt-perm && mkdir /tmp/kt-perm
> touch /tmp/kt-perm/f

# --- chmod numeric -----------------------------------------------------

> chmod 644 /tmp/kt-perm/f
> ls -l /tmp/kt-perm/f
~ -rw-r--r--.*f

> chmod 755 /tmp/kt-perm/f
> ls -l /tmp/kt-perm/f
~ -rwxr-xr-x.*f

> chmod 600 /tmp/kt-perm/f
> ls -l /tmp/kt-perm/f
~ -rw-------.*f

# --- chmod symbolic ----------------------------------------------------

> chmod u+x /tmp/kt-perm/f
> ls -l /tmp/kt-perm/f
~ -rwx

> chmod a-x /tmp/kt-perm/f
> ls -l /tmp/kt-perm/f
~ -rw-

# --- chown -------------------------------------------------------------

> chown 0:0 /tmp/kt-perm/f
> echo chown_rc=$?
< chown_rc=0

> ls -l /tmp/kt-perm/f
~ 0\s+0

# --- chgrp -------------------------------------------------------------

> chgrp 0 /tmp/kt-perm/f
> echo chgrp_rc=$?
< chgrp_rc=0

# --- cleanup -----------------------------------------------------------

> rm -rf /tmp/kt-perm
