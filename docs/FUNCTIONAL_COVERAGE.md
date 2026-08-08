# Functional coverage report

This report distinguishes route declaration coverage from functional handler coverage. `ROUTE_COVERAGE.md` answers whether old URLs exist; this file tracks routes still deliberately mapped to placeholders or broad stubs.

Total Rust route declarations: **146**
Routes still mapped to `legacy::not_implemented`: **0**
Routes still mapped to `stub_admin`: **0**

## v4 implementation notes

- Replaced the remaining explicit placeholders for `/activate`, `/activate.jsp`, `/addphoto.jsp` and `/deregister.jsp` with real handlers.
- Fixed `/check-login`: it now matches the original registration AJAX endpoint and checks `nick` availability instead of returning current-session state.
- Added HMAC-SHA256 activation code verification compatible with the original `SecretTokenService` formula when `SITE_SECRET` matches the Java secret.
- Added userpic upload with multipart parsing, size/dimension checks and `/photos/*` static serving.
- Added destructive deregistration flow: password check, required confirmations, profile cleanup and user blocking.
- Replaced admin stubs with basic moderation handlers for GeoIP lookup surface, search reindex queue acknowledgement, IP bans, group editing, user moderation actions and warnings.
- Fixed `monthly_stats` compatibility migration to match the original demo schema columns.

## Remaining functional gaps

There are no routes left that intentionally return `legacy::not_implemented`, but full production parity still requires endpoint-specific compatibility work for the larger subsystems below:

- real SMTP delivery and registration/password-reset email workflow;
- captcha and anti-flood checks;
- full Spring Security role model, persistent remember-me sessions and CSRF parity;
- production image processing pipeline, animated image detection and object storage/CDN behavior;
- real search reindex backend instead of an admin acknowledgement page;
- MaxMind/GeoIP database integration;
- exact moderator audit log/user-log semantics;
- full notification/tracker/realtime event generation.

## v5 additions

- Fixed current Java poll compatibility: `polls`, `polls_variants`, `vote_users.variant_id` and POST `/vote.jsp` semantics.
- Added current Java account settings/audit surfaces: `user_settings`, `user_log_action`, `user_log`, and a Rust audit helper.
- Compatibility scripts now run Python tools through `python3`, so the suite works even when archive extraction drops executable bits.
