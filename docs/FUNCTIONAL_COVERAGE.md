# Functional coverage report

This report distinguishes route declaration coverage from functional handler coverage. `ROUTE_COVERAGE.md` answers whether old URLs exist; this file tracks which Rust routes still deliberately fall through to `legacy::not_implemented`.

Total Rust route declarations: **146**
Routes with non-placeholder handlers: **142**
Routes still mapped to `legacy::not_implemented`: **4**

## Remaining placeholder routes

| Methods | Path | Handler |
|---|---|---|
| `GET,POST` | `/activate` | `get(legacy::not_implemented).post(legacy::not_implemented)` |
| `GET,POST` | `/activate.jsp` | `get(legacy::not_implemented).post(legacy::not_implemented)` |
| `GET,POST` | `/addphoto.jsp` | `get(legacy::not_implemented).post(legacy::not_implemented)` |
| `GET,POST` | `/deregister.jsp` | `get(legacy::not_implemented).post(legacy::not_implemented)` |

## v3 implementation notes

- Added functional handlers for old redirect endpoints: `/group.jsp`, `/group-lastmod.jsp`, `/view-section.jsp`, `/view-news.jsp`.
- Added archive pages for section archives and month archives.
- Added compatibility handlers for `/markup/preview`, `/check-login`, `/yandex-tableau`, `/show-comments.jsp`, `/show-replies.jsp`.
- Added read/write skeletons for memories, reactions, votes, tag moderation, user filters, deleted views and basic profile/settings/remarks flows.
- Left activation, photo upload and account deregistration as explicit placeholders because the original behavior depends on email tokens, upload storage and destructive account policy.
