# Email compatibility

The current Java application submits RFC 5321 mail to an unauthenticated SMTP
server at `localhost:25`. The Rust port now uses the same default and exposes
`SMTP_HOST`, `SMTP_PORT` and `SMTP_HELO_NAME` so a container can reach the same
MTA used by the existing installation.

Implemented flows:

- new-account activation mail;
- changed-email activation mail;
- password-reset code mail.

The activation/reset HMAC inputs and the user-visible mail text follow the
current Java `EmailService`. Password-reset codes are never rendered into the
HTTP response. A reset timestamp and its `sent_password_reset` audit record are
written only after the SMTP server has accepted the message, matching Java's
ordering.

SMTP failures are request failures, not silent success. Registration itself is
already committed before Java sends its activation message, so Rust preserves
that ordering too: a failed registration email leaves the unactivated account
available for an operational resend/recovery workflow.

Still to port:

- the asynchronous administrator exception-report mailbox/actor;
- explicit resend UI and delivery observability;
- a container-local development MTA (production should point at the existing
  MTA instead of silently discarding mail).
