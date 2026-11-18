# Architecture — request flows

This is the companion to the [README](../README.md). Where the README states the decisions, this file traces the paths
through the system, including the ugly ones, because the failure paths are the design.

Component view is in the README. Below: the three flows that matter, then the failure table.

---

## Flow 1 — Pedestal control (the common path)

```
iOS app
  │ POST /v1/stations/{id}/toggle   Authorization: Bearer <token>
  ▼
API Gateway (HTTP API) ──► Lambda
                            │
                            ├─ 1. token → identity           (no session store needed)
                            ├─ 2. permission check            (does this identity own this berth?)
                            ├─ 3. rate limit                  (per identity + per station)
                            ├─ 4. route to the marina's legacy backend address
                            ├─ 5. call legacy backend: read current state, write new state
                            └─ 6. audit record → DynamoDB     (who toggled what, when, from where)
```

Design notes:

- The legacy backend is the **system of record** for device state. The middleware does not cache the state it reports
  for control; a stale "on" is worse than a slow "unknown".
- The per-marina address comes from configuration, not from the request. A client cannot ask the middleware to talk to
  an arbitrary host — that would be an SSRF primitive.
- The identity is derived from the token, never from a body field. This is the difference between "I can toggle my own
  berth" and "I can toggle anyone's berth".

## Flow 2 — Top-up payment, happy path

```
iOS app ──► POST /payments/topup ──► Lambda
                                       │
                                       ├─ write PENDING session  ──────► DynamoDB
                                       │    (basket_id, conversation_id,
                                       │     expected amount, expires_at)
                                       │
                                       ├─ create checkout form  ───────► payment provider
                                       │
                                       └─ return checkout URL + token ──► iOS app
                                              │
                                              ▼
                                     hosted checkout page (provider)
                                              │
                        ┌─────────────────────┴──────────────────────┐
                        │                                            │
              browser callback                              server webhook
        POST /payments/callback?token=…              POST /payments/webhook
                        │                                            │
                        └───────────────┬────────────────────────────┘
                                        ▼
                              SAME consume routine
                                        │
                        ┌───────────────┼────────────────┐
                        │               │                │
                verify signature   claim the session   (conditional write)
                (webhook only)     ─────────────────►  winner calls legacy
                        │                                to credit balance
                        │                                  │
                        └──────────────────────────────────┴─► terminal state
                                                               COMPLETED / REJECTED
                                                               response stored
```

The point of the shape: **two entry points, one consume routine.** Neither is privileged. Whichever arrives first
claims the session and does the work; the other one finds a terminal record and returns the *stored* response.

Checks performed before any credit, all of them explicit (see [ADR 0002](adr/0002-idempotency-outside-the-legacy-system.md)):

| Check | Failure mode it prevents |
|---|---|
| signature valid (webhook path) | forged "payment succeeded" |
| session exists and not expired | replay of an old callback with a new body |
| `basket_id` matches the session | credit for a different purchase |
| `conversation_id` matches the session | provider-side mixup between two payments |
| status is a success status | treating a pending/failed payment as success |
| amount matches the expected amount | partial payment credited in full, or currency confusion |

## Flow 3 — Webhook arrives before the callback, twice

```
t0  webhook #1  ─► verify sig ─► claim PENDING ─► SUCCESS ─► credit ─► COMPLETED (response stored)
t1  webhook #2  ─► verify sig ─► claim fails (already COMPLETED) ─► return stored response, no write
t2  callback    ─► load session ─► already COMPLETED ─► return stored response, no write
```

Duplicate cost after the change: **one extra read**. Before the change: a second balance credit.

## Failure table

| Failure | Detected by | Behaviour | Recovery |
|---|---|---|---|
| Signature mismatch | constant-time HMAC verify | 401, nothing parsed, nothing written | none needed; the real webhook will arrive |
| Missing signature header | presence check before verify | 401 | none needed |
| Session not found | read path | 404 + log with token | user retries the top-up |
| Session expired | `expires_at` check on read (see [ADR 0003](adr/0003-dynamodb-ttl-is-not-a-correctness-mechanism.md)) | 410, no credit | user retries the top-up |
| Duplicate while in flight | conditional claim returns "not mine" | 409 "in progress" | provider retries later; second attempt sees terminal state |
| Amount/basket mismatch | explicit field comparison | reject + audit record with reason | manual review; better a stuck payment than a wrong balance |
| Legacy backend 5xx after claim | upstream error | 5xx **after** claim; session left claimed | reconciliation job: `PENDING` older than N minutes |
| Legacy backend timeout (28s) | request timeout | 504, unknown upstream outcome | reconciliation job; this is the documented crash window |
| KMS unavailable | decrypt error | 503 with a typed error, feature degraded not bypassed | none; do not fall back to a weaker scheme |
| SSM unavailable at cold start | cold-start init error | function fails to initialise, invocation errors | retry; the platform re-attempts cold start |
| Rate limit exceeded | per-identity limiter | 429 with retry hint | client backs off |

## What the diagram deliberately does not show

Per the sanitisation rules in [what-is-not-here.md](what-is-not-here.md): no account identifiers, no real hostnames, no
table names, no parameter paths, no role names. The *shape* is the content; the plumbing is my employer's.
