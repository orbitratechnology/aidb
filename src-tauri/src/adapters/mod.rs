//! Adapters perform I/O for the domain: the local metadata store, the OS credential store,
//! user-chosen files and PostgreSQL today; AI providers as that feature lands.

pub mod credentials;
pub mod database;
pub mod files;
pub mod local_store;
pub mod postgres;
