//! Adapters perform I/O for the domain: the local metadata store, the OS credential store,
//! user-chosen files, PostgreSQL and AI providers.

pub mod ai_providers;
pub mod credentials;
pub mod database;
pub mod files;
pub mod local_store;
pub mod postgres;
