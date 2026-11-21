//! The policy engine: how risky a statement is, and what must happen before it may run.
//! Deterministic and versioned; nothing here trusts a model's or the UI's label.

pub mod approval;
pub mod classify;
pub mod facts;
pub mod risk;
