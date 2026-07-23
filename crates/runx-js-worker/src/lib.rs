//! Authority-free deterministic JavaScript engine for Runx.

pub use runx_contracts::javascript_worker as protocol;

mod engine;
mod limits;
mod server;

pub use server::serve;
