# 0003 — DynamoDB TTL is a cleanup mechanism, not a correctness mechanism

- **Status:** accepted, shipped
- **Date:** 2026
- **Supersedes:** an earlier assumption that expired items simply disappear

## Context

The middleware stores short-lived records: pending payment sessions, "remember me" markers, temporary device
credentials, revocation entries. All of them have a natural expiry — a checkout session that was abandoned 40 minutes
ago must not be usable any more.

DynamoDB has a TTL feature: set an epoch-seconds attribute, and DynamoDB deletes the item "typically within 48 hours".
That last part is the trap. TTL deletion is **lazy by design**, and AWS documents it as best-effort: an item whose TTL
has passed is still returned by `GetItem` until the deletion actually happens, which can be days later under load.

The first version of the code treated `expires_at` as a promise from the database. It was not. Two concrete effects:

- An abandoned checkout session stayed **valid** well past its lifetime, because the read path simply did not look at
  the timestamp.
- A conditional write that depended on "the item does not exist" was satisfied by an item that logically was gone.

## Decision

Two separate mechanisms, doing two separate jobs:

1. **`expires_at` is a domain field.** Every read path checks it against the current time and treats a passed
   timestamp as *absent*. This is the correctness mechanism.
2. **TTL is a cost control.** The same attribute is registered as the table's TTL so that dead records are eventually
   deleted and storage does not grow without bound. This is the cleanup mechanism.

Written as a rule for the codebase: *if a record has `expires_at`, no code may read it without a liveness check.*
Conditional writes include the expiry predicate rather than assuming absence.

## Alternatives considered

| Alternative | Rejected because |
|---|---|
| Rely on TTL alone | It is documented as delayed; correctness would depend on a deletion scheduler we do not control |
| Delete on read when expired | Turns every read into a write, doubles consumed capacity, and races with the TTL deleter doing the same thing |
| A scheduled job that scans and deletes | Still eventual at a different granularity, plus a scanner to operate |
| No expiry; explicit invalidation | Requires an invalidation path for *every* abandonment case, including the app being killed mid-checkout |
| Strongly consistent reads to avoid stale items | `ConsistentRead` does not make TTL deletion synchronous; it solves a different problem |

## Consequences

**Good**

- Expiry is deterministic and testable: the read path can be unit-tested with a fake clock, without touching DynamoDB.
- Storage still stays bounded, with no scanner to operate.
- The same pattern applies to every record type in the table, so there is one rule instead of many.

**Bad / accepted costs**

- **One extra comparison on every read**, and a discipline cost: a new read path that forgets the check is a silent
  bug. Mitigated by routing all record access through a single typed layer (`kv`-style helpers) so the check lives in
  one place, and by documenting the rule at the top of that module.
- **Dead records are visible to anything that bypasses the layer** — for example a hand-run query during an incident.
  Expiry is an application invariant, not a database invariant.
- Audit records need a *different* TTL from operational records; sharing one table means the two lifetimes live side
  by side and must not be confused.

## What I would do differently

Encode liveness in the type system instead of in a convention: return something like `Option<LiveRecord<T>>` from the
access layer so that "expired" is not representable, and callers cannot forget. That converts a rule someone has to
remember into a compile error, which is the only kind of rule that survives a deadline.
