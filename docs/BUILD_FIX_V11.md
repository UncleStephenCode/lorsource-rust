# Build fix v11

This iteration fixes the first Docker `cargo build --release` failure reported during `docker compose up`.

## Fixed

- Removed the unused Askama `with-axum` feature from `Cargo.toml`.
  The project renders templates explicitly via `Template::render()` and returns `axum::response::Html<String>`, so the `askama_axum` integration crate is not required.
- Added missing `axum::Form` import in `src/routes/users.rs` for handlers that destructure `Form(form)`.

## Original errors

- `error[E0433]: cannot find askama_axum in the crate root`
- `error[E0531]: cannot find tuple struct or tuple variant Form in this scope`

## Notes

The sandbox used for preparing this archive does not provide `cargo`, `rustc` or Docker, so the fix is based on the compiler diagnostics from the Docker build log and static source inspection.
