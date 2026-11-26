//! Monotonic clock: `std::time::Instant` panics on wasm32-unknown-unknown
//! ("time not implemented on this platform"), so the web build uses the
//! browser's performance clock through `web-time`.

#[cfg(target_arch = "wasm32")]
pub use web_time::Instant;

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;
