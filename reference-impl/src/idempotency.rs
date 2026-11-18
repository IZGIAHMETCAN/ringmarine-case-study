//! An idempotency store with single-flight semantics and explicit expiry.
//!
//! The problem this models: a payment is announced through two independent channels (a browser redirect and a server
//! webhook), delivery is at-least-once, and the system of record behind us has no transactions, no conditional writes
//! and no way to ask "did I already apply this?".
//!
//! So the claim lives here, in front of the system of record:
//!
//! ```text
//! begin(key)
//!   ├── Start(claim)      we are the winner: do the work, then claim.complete(response)
//!   ├── InProgress        someone else is doing it right now: answer 409, do not do it again
//!   └── Completed(resp)   it is done: return the stored response, do not do it again
//! ```
//!
//! Three details that are easy to get wrong and are the reason this module exists:
//!
//! * **Expiry is checked on read.** A record past its `expires_at` is treated as absent, because DynamoDB's TTL
//!   deletion is lazy and can be days late. See ADR 0003 in the case study.
//! * **Dropping a claim releases it.** If the handler panics or returns early, the claim must not stay locked forever;
//!   a retry has to be able to proceed.
//! * **The stored response is returned to duplicates.** A retry should get the same answer, not a different one — that
//!   is what makes the operation idempotent from the caller's point of view, not just from the balance's.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What a completed operation answered, so duplicates can be answered identically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Debug)]
enum RecordState {
    InFlight,
    Done(StoredResponse),
}

#[derive(Debug)]
struct Record {
    state: RecordState,
    expires_at: i64,
}

#[derive(Debug)]
struct Inner {
    records: Mutex<HashMap<String, Record>>,
    ttl_secs: i64,
}

/// The result of asking to begin an operation.
#[derive(Debug)]
pub enum Begin {
    /// We won the claim. The work may proceed; the claim is released on drop unless completed.
    Start(Claim),
    /// Another delivery of the same operation is being processed right now.
    InProgress,
    /// The operation already completed. Return this response verbatim.
    Completed(StoredResponse),
}

/// Permission to perform the operation, for exactly one caller.
///
/// Dropping this without calling [`Claim::complete`] abandons the claim, which is what you want on an error path: the
/// next delivery gets a fresh chance instead of being told "in progress" forever.
#[derive(Debug)]
pub struct Claim {
    inner: Arc<Inner>,
    key: String,
    finished: bool,
}

impl Claim {
    /// Record the terminal response and release the claim.
    pub fn complete(mut self, response: StoredResponse, now: i64) {
        {
            let mut records = self.inner.records.lock().expect("lock poisoned");
            if let Some(record) = records.get_mut(&self.key) {
                record.state = RecordState::Done(response);
                record.expires_at = now + self.inner.ttl_secs;
            }
        }
        self.finished = true;
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let mut records = self.inner.records.lock().expect("lock poisoned");
        records.remove(&self.key);
    }
}

/// An in-memory stand-in for the real store. The production version is the same logic over DynamoDB conditional
/// writes (`attribute_not_exists` on the claim, `expires_at` as the TTL attribute); the trait boundary is deliberately
/// absent here to keep the reference small, but a real implementation would sit behind one.
#[derive(Debug, Clone)]
pub struct IdempotencyStore {
    inner: Arc<Inner>,
}

impl IdempotencyStore {
    /// `ttl_secs` is how long a *completed* record is reused before a repeat delivery may do the work again.
    pub fn new(ttl_secs: i64) -> Self {
        Self {
            inner: Arc::new(Inner {
                records: Mutex::new(HashMap::new()),
                ttl_secs,
            }),
        }
    }

    /// Ask to begin the operation identified by `key`, as of `now` (unix seconds).
    pub fn begin(&self, key: &str, now: i64) -> Begin {
        let inner = &self.inner;
        let mut records = inner.records.lock().expect("lock poisoned");

        // Expiry is a correctness check performed on read, not a promise from the storage layer.
        if records
            .get(key)
            .map(|record| record.expires_at <= now)
            .unwrap_or(false)
        {
            records.remove(key);
        }

        // Decide first, mutate after: the borrow from `get` must end before `insert`/`remove`.
        enum Decision {
            Fresh,
            InFlight,
            Done(StoredResponse),
        }

        let decision = match records.get(key) {
            None => Decision::Fresh,
            Some(Record {
                state: RecordState::InFlight,
                ..
            }) => Decision::InFlight,
            Some(Record {
                state: RecordState::Done(response),
                ..
            }) => Decision::Done(response.clone()),
        };

        match decision {
            Decision::Fresh => {
                records.insert(
                    key.to_string(),
                    Record {
                        state: RecordState::InFlight,
                        expires_at: now + inner.ttl_secs,
                    },
                );
                Begin::Start(Claim {
                    inner: Arc::clone(inner),
                    key: key.to_string(),
                    finished: false,
                })
            }
            Decision::InFlight => Begin::InProgress,
            Decision::Done(response) => Begin::Completed(response),
        }
    }
}
