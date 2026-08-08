#!/bin/sh
set -eu

# Named upload volumes created by older images can still belong to root. Fix
# only this dedicated writable directory, then ensure the web process itself
# never runs with container-root privileges.
if [ "$(id -u)" = "0" ]; then
    chown -R lorsource:lorsource /app/uploads
    exec gosu lorsource:lorsource "$@"
fi

exec "$@"
