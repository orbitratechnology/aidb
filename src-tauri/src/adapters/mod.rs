//! Adapters perform I/O for the domain: the local metadata store, the OS credential store and
//! user-chosen files today; PostgreSQL and AI providers as those features land.

pub mod credentials;
pub mod files;
pub mod local_store;
