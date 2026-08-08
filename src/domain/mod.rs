//! Domain layer.
//!
//! The original port kept unrelated structs in `models.rs`.  This layer follows
//! the geoip-style split into small domain modules: HTTP routes depend on domain
//! contracts, and PostgreSQL-specific SQL is pushed down into `infra`.

pub mod comment;
pub mod common;
pub mod forum;
pub mod tag;
pub mod topic;
pub mod user;
