# What is deliberately not in this repository

This repository describes a production system I built as an employee. Publishing it required a line, and this
file states where that line is, so that nothing here is ambiguous.

## Not published

| Excluded | Why |
|---|---|
| The middleware source code (~10k lines of Rust) | It is my employer's product, not mine to publish |
| Infrastructure as code (SAM template, deploy scripts) | Contains account identifiers, role names, parameter paths, route IDs |
| Deployment artefacts (task definitions, rollback dumps) | Same: internal identifiers with no explanatory value |
| Configuration values | Everything is resolved at runtime from a managed parameter store |
| Customer and marina names, hosts, addresses | Third-party data |
| Payment-provider credentials and callback URLs | Credentials, obviously; the URLs are per-environment |
| Database table names and key layouts in detail | Design is described; the exact physical layout is not |

## Published instead

1. **The problem and the constraints.** Anyone can copy an architecture diagram; the constraints are what make a
   design defensible or not.
2. **Five decision records**, each with the alternatives I rejected and the cost I accepted.
3. **What I would do differently.** The design's real weaknesses, written by me.
4. **`reference-impl/`** — a small, independent Rust crate that demonstrates the two patterns where the risk was
   highest. Written from scratch for this repository: generic HMAC request signing with a replay window, and an
   idempotency store with single-flight semantics. It is *inspired by* the production problems and shares no
   code with them.

## Sanitisation checklist used before publishing

- [x] No AWS account IDs, ARNs, or resource names
- [x] No hostnames, IP addresses, or internal endpoints
- [x] No API keys, tokens, secrets, or private keys
- [x] No bearer tokens or session identifiers from real traffic
- [x] No customer, marina, or vessel names
- [x] No third-party data of any kind
- [x] Grep sweep for `AKIA`, `arn:aws`, 12-digit numbers, `secret`, `password`, `token`, private-key blocks

If you believe something here still crosses that line, please open an issue and I will remove it immediately.
