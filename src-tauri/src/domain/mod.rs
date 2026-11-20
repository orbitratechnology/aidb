//! Domain types and rules. No I/O happens here: adapters persist these types and services
//! coordinate them.

pub mod advisor;
pub mod agent;
pub mod ai;
pub mod audit;
pub mod catalog;
pub mod connection_transfer;
pub mod connection_uri;
pub mod connections;
pub mod csv_import;
pub mod data_change;
pub mod database;
pub mod export;
pub mod library;
pub mod migration;
pub mod policy;
pub mod providers;
pub mod query;
pub mod schema_diff;
pub mod settings;
pub mod sql;
pub mod table_data;
