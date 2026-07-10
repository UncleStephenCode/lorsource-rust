# Profile and theme parity v15

This iteration makes the Rust port closer to the visible Java/Spring LOR instance.

## Implemented

- Reworked `/people/{nick}/profile` from a minimal stub into a whois-like profile page.
- Added user card fields compatible with the Java `whois.jsp` surface:
  - Nick / ID / name / URL / town / registration date / last login;
  - status flags: moderator, administrator, corrector, blocked;
  - private email and score/maxScore for owner/moderator;
  - userpic block and userinfo block;
  - favorite and ignored tags;
  - owner actions: edit profile, settings, logout, user-filter, favs, tracked, drafts, remarks;
  - basic topic/comment statistics.
- Reworked `/people/{nick}/edit` into a closer analogue of `edit-profile.jsp`:
  - name, URL, email, town, userinfo;
  - optional password/password2 update with the same minimum length policy as registration.
- Reworked `/people/{nick}/settings` into a closer analogue of `edit-settings.jsp`:
  - photos, hideAdsense, mainGallery, oldTracker, reactionNotification;
  - style selection;
  - topics/messages page sizes;
  - tracker mode;
  - avatar mode;
  - markup mode.
- Added `src/profile.rs` with Java-compatible default profile keys from `DefaultProfile.scala`.
- Added original Java webapp static assets under `static/` and root-compatible `ServeDir` mounts:
  - `/img`, `/font`, `/js`, `/black`, `/tango`, `/white2`, `/waltz`, `/zomg_ponies`, `/adv`.
- Added `static/theme-lor.css` with a runtime theme compatibility layer.
- Updated `templates/base.html` to use the original-style `LINUX.ORG.RU` header and theme bootstrap script.

## Notes

The original Java themes are SCSS/JSP-driven. The Rust port now serves the original assets and provides compatible theme IDs, but full SCSS compilation and exact JSP header parity are still separate work.
