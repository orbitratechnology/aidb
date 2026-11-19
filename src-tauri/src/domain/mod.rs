//! Domain types and rules. No I/O happens here: adapters persist these types and services
//! coordinate them.

pub mod agent;
pub mod ai;
pub mod catalog;
pub mod connection_transfer;
pub mod connection_uri;
pub mod connections;
pub mod database;
pub mod export;
pub mod library;
pub mod providers;
pub mod query;
pub mod settings;
pub mod sql;
pub mod table_data;
