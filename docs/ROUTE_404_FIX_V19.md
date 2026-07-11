# Route 404 compatibility fix v19

Fixed the common class of 404s reported during manual testing:

- `/forum/` returned 404 while the Java/Spring site accepts the section URL and the template links to `/forum/`.
- `/people/<nick>/profile` and `/people/<nick>/settings` could be shadowed/handled inconsistently because the short `/people/{nick}` route was declared before user sub-pages.
- Unknown legacy URLs with a trailing slash now redirect to the canonical URL without the trailing slash instead of returning a misleading 404.

The Rust port now keeps user sub-page routes before `/people/{nick}` and normalizes trailing slash paths in the fallback handler.
