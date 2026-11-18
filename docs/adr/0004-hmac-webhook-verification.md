# 0004 — Verify webhook signatures in constant time, before trusting the body

- **Status:** accepted, shipped
- **Date:** 2026

## Context

There is a public, unauthenticated HTTPS endpoint in this system whose only job is to receive "this payment
succeeded" messages. It has no bearer token, no session, and no caller identity to check. Anyone on the internet can
POST to it.

What makes it safe is that the payment provider signs the payload with a shared secret:

- the canonical message is a **concatenation of specific fields in a specific order** (event type, payment id, token,
  conversation id, status) prefixed by the secret,
- the signature is `HMAC-SHA256`, hex-encoded, and sent in a header,
- the secret is the same one used for API calls.

Three ways this was plausibly wrong before it was right:

1. Parsing the body first and verifying afterwards, so business logic could see unverified data.
2. Comparing signatures with `==`, which short-circuits and leaks timing information about how many leading bytes
   matched.
3. Treating a missing signature as "the provider did not sign this one", and proceeding.

All three are wrong on the same endpoint that moves money.

## Decision

**Verify first, trust afterwards.** In order:

1. Read the body with a hard size limit, so a hostile request cannot exhaust memory before signature verification runs.
2. Reject if the signature header is absent. Never optional.
3. Decode the hex **and reject malformed hex as an error**, rather than silently comparing a mangled buffer.
4. Recompute `HMAC-SHA256(secret, canonical_string)` and compare with **`verify_slice`**, which is constant time and
   also checks the length for free.
5. Only after that does any deserialisation into business types happen.
6. Respond `2xx` quickly, because the provider retries non-2xx for hours and every retry is a duplicate the idempotency
   layer (ADR 0002) must then absorb.

Two supporting rules fell out of this:

- **The canonical form is pinned by tests.** The provider's concatenation order is not something to infer from docs at
  runtime; there is a test per field that asserts changing that field invalidates the signature.
- **Numbers in signed responses are formatted the provider's way.** One signature covers a *retrieve-response* whose
  price fields must be rendered with trailing zeros trimmed (`120.50` → `120.5`, `120.00` → `120`). Getting this wrong
  produces a signature mismatch that looks like a wrong secret, and cost real debugging time.

## Alternatives considered

| Alternative | Rejected because |
|---|---|
| IP allowlist of the provider | Fragile, breaks on their infrastructure changes, and gives no integrity guarantee |
| mTLS | Not offered by the provider for webhooks |
| Verify with `==` | Timing side channel on a money-moving endpoint; `verify_slice` is free |
| Verify after parsing | Business logic would run on unauthenticated input; also invites "parse errors" masking signature errors |
| Shared bearer token instead of HMAC | Does not bind the signature to the payload, so a valid token can be replayed with a different body |
| Rely on the callback path only and disable the webhook | The callback can be lost (app killed, redirect failed); the webhook is the reliable channel |

## Consequences

**Good**

- The endpoint is genuinely safe to expose to the internet; the signature is the authentication.
- Constant-time comparison removes an entire class of "did you know about timing attacks?" findings.
- Because verification happens before parsing, error reporting cannot leak parsed structure.

**Bad / accepted costs**

- **The canonical string is provider-idiomatic and undocumented in detail.** It is effectively a small protocol to
  maintain, and a provider-side change would break it silently. Pinned by tests, which is the cheapest available
  insurance.
- **Synchronous verification adds latency** to every webhook (two hashes). Negligible against the network round trip.
- **Replay is only partially mitigated.** HMAC proves authenticity, not freshness. Duplicates are handled downstream by
  the idempotency claim, not here. A timestamp-in-signature scheme would be better, and the provider does not offer one.

## What I would do differently

Add a **per-event monotonic dedupe key** derived from the signed fields, and record it before responding `2xx`, so that
"the provider retried the same event 30 times" is visible as one row with a retry count, instead of 30 log lines and a
manual reconciliation. The signature tells you it is real; it does not tell you whether you have already seen it.
