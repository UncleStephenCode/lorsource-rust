#!/bin/sh
set -eu

stage_secret_file() {
    sName="$1"
    sFileVariable="${sName}_FILE"
    eval "bDirectSet=\${${sName}+x}"
    eval "sSource=\${${sFileVariable}-}"
    if [ "${bDirectSet:-}" = "x" ] && [ -n "$sSource" ]; then
        echo "Set only one of $sName and $sFileVariable" >&2
        exit 1
    fi
    if [ -z "$sSource" ]; then
        return
    fi
    if [ ! -f "$sSource" ]; then
        echo "$sFileVariable does not name a readable file" >&2
        exit 1
    fi

    sSecretDirectory=/tmp/lorsource-secrets
    sTarget="$sSecretDirectory/$sName"
    mkdir -p "$sSecretDirectory"
    chown root:root "$sSecretDirectory"
    chmod 0700 "$sSecretDirectory"
    cp -- "$sSource" "$sTarget"
    chmod 0400 "$sTarget"
    chown lorsource:lorsource "$sTarget"
    chmod 0500 "$sSecretDirectory"
    chown lorsource:lorsource "$sSecretDirectory"
    export "$sFileVariable=$sTarget"
}

# Named upload volumes created by older images can still belong to root. Fix
# only this dedicated writable directory, then ensure the web process itself
# never runs with container-root privileges.
if [ "$(id -u)" = "0" ]; then
    for sName in DATABASE_URL COOKIE_SECRET SITE_SECRET CAPTCHA_PRIVATE_KEY TELEGRAM_TOKEN; do
        stage_secret_file "$sName"
    done
    if [ "${SKIP_UPLOAD_CHOWN:-false}" != "true" ]; then
        chown -R lorsource:lorsource /app/uploads
    fi
    exec gosu lorsource:lorsource "$@"
fi

exec "$@"
