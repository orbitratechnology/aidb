//! Adapters perform I/O for the domain: the local metadata store and the OS credential store
//! today; PostgreSQL and AI providers as those features land.

pub mod credentials;
pub mod local_store;
