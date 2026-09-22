//! Route-B authoritative store boundary.
//!
//! This module is not a general SQL service. B1 binds the fixed SQLite image.
//! B2 CommitDomainRecord is the first typed complete write group on that
//! same-open connection. Caller SQL is not admitted.

pub mod digest;
pub mod sqlite_identity;

#[cfg(windows)]
pub mod same_open;

#[cfg(windows)]
pub mod atomic;

#[cfg(windows)]
pub mod action;

#[cfg(windows)]
pub mod context;

#[cfg(windows)]
mod context_graph;

#[cfg(all(test, windows))]
mod context_graph_tests;

#[cfg(windows)]
mod context_state;

#[cfg(all(test, windows))]
mod context_state_tests;

#[cfg(windows)]
pub(crate) mod authority;

#[cfg(windows)]
pub mod donor_migrations;

#[cfg(windows)]
pub mod migrate;

#[cfg(windows)]
pub mod orchestration;

pub mod protocol;

#[cfg(windows)]
pub mod session;

#[cfg(windows)]
pub mod product_database;
