//! Authority-free deterministic JavaScript engine for Runx.

pub use runx_contracts::javascript_worker as protocol;

#[cfg(feature = "engine")]
mod engine;
#[cfg(feature = "engine")]
mod limits;
#[cfg(feature = "engine")]
mod server;

#[cfg(feature = "engine")]
pub use server::serve;
