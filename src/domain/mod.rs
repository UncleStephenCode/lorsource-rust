//! Domain layer.
//!
//! The original port kept unrelated structs in `models.rs`.  This layer follows
//! the geoip-style split into small domain modules: HTTP routes depend on domain
//! contracts, and PostgreSQL-specific SQL is pushed down into `infra`.

pub mod boxlet;
pub mod comment;
pub mod common;
pub mod email;
pub mod email_domain_block;
pub mod forum;
pub mod realtime;
pub mod tag;
pub mod topic;
pub mod user;
pub mod warning;
