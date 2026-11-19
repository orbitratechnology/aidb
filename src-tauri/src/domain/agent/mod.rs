//! The natural-language assistant's domain: what a task plan is, which tools the model may call,
//! and what context it is given. No I/O here; the planner loop (services) runs the tools.
//!
//! The model proposes; Rust decides. Risk, required approval, the target connection and every
//! tool's arguments are checked or computed here, never taken from the model's say-so.

pub mod context;
pub mod plan;
pub mod risk;
pub mod tools;
