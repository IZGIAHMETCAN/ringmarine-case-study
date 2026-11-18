# reference-impl — webhook verification and idempotent payment state

A small, dependency-light Rust crate demonstrating the two patterns that carry the most risk in the system described
in the [parent case study](../README.md). **This is not production code.** It shares no code with the production
system: it is an independent implementation, written to make the trade-offs readable and testable.

```bash
cargo test      # 12 tests, no external services
```

## Why these two patterns

A payment page has a public, unauthenticated endpoint that credits money when it is told to. Two things stand between
that endpoint and a double charge:

1. **Authenticity** — the request must be provably from the provider (`webhook.rs`).
2. **Idempotency** — the same payment announced twice must produce one effect (`idempotency.rs`).

Getting either wrong compounds: a forged webhook is a free credit, and a duplicate webhook is a paid-for credit twice.
The [ADR set](../docs/adr/) explains the decisions in context.

## `webhook.rs`

```rust
let verifier = Verifier::new(secret, /* tolerance_secs = */ 300)?;

// Order matters: nothing below is deserialised until this returns Ok.
verifier.verify(&SignedRequest {
    body: &raw_body,
    signature_hex: header_value,   // Option<&str>: absent and invalid are different failures
    timestamp,
}, now)?;
```

Four decisions worth reading the source for:

| Decision | Why |
|---|---|
| `verify_slice` instead of `==` | Constant time; no timing oracle revealing how many leading bytes matched |
| Timestamp is **inside** the signed message | Otherwise the replay window is decorative — a captured signature could be re-paired with a fresh timestamp |
| Missing signature is an error, not a pass | The most common real-world bug in this class of endpoint |
| Malformed hex is its own error | "Wrong secret" and "garbage in the header" produce different operations work |
| `Verifier` has no `Debug` impl | It holds the secret; a derived `Debug` would print it into a panic message or log line |

The signed message is `webhook.v1.{timestamp}.{body}` — the `webhook.v1` prefix is a domain separator, so a signature
minted for one purpose cannot be replayed into another.

## `idempotency.rs`

```rust
match store.begin("payment:conv-123", now) {
    Begin::Start(claim) => {
        // We are the winner. Do the work, then record the answer.
        let response = credit_the_balance().await?;
        claim.complete(response, now);
    }
    Begin::InProgress => { /* 409: another delivery is doing it right now */ }
    Begin::Completed(response) => { /* return the same answer the first caller got */ }
}
```

Three decisions worth reading the source for:

| Decision | Why |
|---|---|
| Expiry is checked **on read** | DynamoDB's TTL deletion is lazy and can be days late. TTL bounds storage cost; it does not bound correctness (see [ADR 0003](../docs/adr/0003-dynamodb-ttl-is-not-a-correctness-mechanism.md)) |
| Dropping a `Claim` releases it | An error path must not leave the payment locked in "in progress" forever — the next delivery has to get a chance |
| Duplicates get the **stored response** | Idempotency means the caller sees the same answer, not just that the balance was touched once |

The in-memory store stands in for DynamoDB. In production the same states are expressed as conditional writes
(`attribute_not_exists` for the claim), and `expires_at` is registered as the table's TTL attribute — the read path
still checks it, because that is the correctness mechanism.

## What this crate deliberately does not do

- **No HTTP server.** The patterns are the point; wiring them into `axum` is a few lines and would distract from the
  ordering rules that matter.
- **No real crypto review.** The HMAC usage follows the `hmac` crate's API correctly, but this is a demonstration of
  a protocol, not a substitute for an audit.
- **No clock.** Both modules take `now` as a parameter, which is why expiry and replay are unit-testable without
  sleeping or mocking a clock.

## License

MIT — see [../LICENSE](../LICENSE).
