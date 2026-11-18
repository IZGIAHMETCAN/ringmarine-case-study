# 0002 — Own the idempotency boundary in the middleware, not in the legacy backend

- **Status:** accepted, shipped
- **Date:** 2026

## Context

Money arrives through a payment provider that can tell us about the same payment **twice, three times, or in the
wrong order**, through **two different channels**:

1. a **browser callback** — the user's phone returns from the hosted checkout page with a token;
2. a **server webhook** — the provider posts the result to us, independently, possibly minutes later, possibly
   after the callback already succeeded.

The provider is explicit that delivery is *at least once*. Duplicates are normal operation, not an error.

Behind the middleware sits a legacy marina back-office which is the system of record for balances. It has:

- no transactions,
- no conditional writes,
- no idempotency keys,
- no way to ask "did I already apply this top-up?".

So "just retry, it's fine" was not available: retrying *was* the double charge. We had already seen a berth credited
twice from a single payment.

## Decision

The middleware owns an **idempotency boundary in front of the legacy backend**, keyed on the payment's
`conversation_id`/`basket_id` pair, with a **pending-session record** as the claim:

1. **Before** redirecting the user to the hosted checkout, write a pending session:
   `PK = SESSION#<checkout_token>`, `SK = PENDING`, carrying `basket_id`, `conversation_id`, the expected amount, and
   an `expires_at`.
2. Both the callback and the webhook arrive at the same **consume** routine. It performs a **conditional claim**
   (write-if-absent / write-if-still-pending) on that record.
3. Only the winner of the claim may call the legacy backend and credit the balance.
4. The record then transitions to a terminal state (`COMPLETED` / `REJECTED`), and the response is stored so that
   duplicates get the *same* answer instead of a different one.
5. A duplicate that arrives while the claim is in flight gets **"in progress"**, not a second credit and not a 500.

Every check that used to be implicit became explicit: amount matches, `basket_id` matches, `conversation_id`
matches, expected status is success, session not expired. Any mismatch is a rejection with a reason, not a silent
credit.

## Alternatives considered

| Alternative | Rejected because |
|---|---|
| Trust the provider to deliver once | It documents the opposite |
| Dedupe inside the legacy backend | Would require changing a system we do not control and cannot deploy atomically |
| Dedupe by storing the last payment id per user | Only catches consecutive duplicates; loses the second payment legitimately made later |
| A queue with exactly-once semantics (SQS FIFO dedupe) | Helps, but the *consumer* must still be idempotent, because the failure we are defending against is at the legacy write, not at the queue |
| Check "is this payment already in my history?" before crediting | A read-then-write race with no conditional write is exactly the bug we were fixing |

## Consequences

**Good**

- Duplicate callbacks and duplicate webhooks now converge on **exactly one** balance update.
- The same mechanism gives free **auditability**: a rejected payment has a record and a reason.
- Callback and webhook can arrive in either order; neither is privileged.

**Bad / accepted costs**

- The claim is a **second write** before the redirect. That is an extra DynamoDB write on every top-up, and it must
  succeed before the user sees the checkout page — a new failure mode ("cannot start payment") that did not exist.
- **A crash between the legacy write and the terminal-state write is a real window.** The claim prevents a *second*
  credit but cannot prove the first one landed. Recovery is manual: reconcile `PENDING` sessions older than N
  minutes against the provider's retrieve endpoint.
- Sessions must expire, which drags in ADR [0003](0003-dynamodb-ttl-is-not-a-correctness-mechanism.md).

## What I would do differently

Write the claim as a **two-phase record** — `CLAIMED` → `APPLIED` — with the upstream call bracketed, so the
reconciliation job has an explicit "we know we called the legacy backend and we do not know the result" state
instead of inferring it from a timeout. That is the outbox pattern, and it is the single change I would insist on
if this system had to handle an order of magnitude more money.
