//! Two patterns that carry most of the risk in a payment-facing service.
//!
//! 1. [`webhook`] — verifying that an unauthenticated HTTP request really came from the provider, in constant time,
//!    before any of its content is trusted.
//! 2. [`idempotency`] — making sure a payment that is announced twice (browser callback and server webhook, duplicated
//!    or out of order) results in exactly one state change.
//!
//! Nothing here is the production code of the system described in the case study. It is an independent, smaller
//! implementation of the same *patterns*, written so the trade-offs can be read and tested.
//!
//! Both modules take the current time as a parameter instead of reading the clock. That is deliberate: expiry and
//! replay windows are the parts that break silently, and they are only testable if time is an input.

pub mod idempotency;
pub mod webhook;
