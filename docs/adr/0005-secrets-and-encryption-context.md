# 0005 — Configuration from a parameter store, sealed values from KMS with a bound context

- **Status:** accepted, shipped
- **Date:** 2026

## Context

Two different kinds of sensitive thing live in this service, and conflating them is how leaks happen:

1. **Configuration and credentials** — the legacy backend address per marina, the payment provider's API key and
   secret, callback URLs. Secrets, but *long-lived* and *identical across every instance*.
2. **Sealed user data** — the "boat card": a credential the boat owner stores on their phone and that we must verify
   server-side without ever storing it in plaintext. Short-lived, per-user, must be decryptable on demand.

The obvious first implementation puts both in Lambda environment variables. That fails for the first kind, because
environment variables are visible in the console, in the function configuration, in any deployment artefact, and in a
screenshot during a support call. It fails for the second kind entirely: there is nothing to encrypt *with*.

The other requirement came from a question worth asking out loud: if a sealed value is decrypted, what stops someone
with a valid ciphertext from using it in a context it was never meant for? Nothing, unless the cryptography is bound to
that context.

## Decision

**Configuration and credentials:** resolve from SSM Parameter Store at cold start, in **one** `GetParametersByPath`
call, with an IAM policy scoped to exactly that parameter path. Nothing sits in a `.env` file in the deployment, nothing
in the container image, nothing in the repository — the example file documents the *names* and holds empty values only.

**Sealed values:** encrypt and decrypt through **KMS**, with an **encryption context** that names the purpose and the
owning entity:

- the context is passed on both `Encrypt` and `Decrypt`; KMS refuses the operation if they differ,
- the IAM policy and the key policy require the `purpose` claim, so a ciphertext minted for one purpose cannot be
  opened by code asking for another,
- the plaintext is returned inside a **zeroizing** buffer, so it is wiped when the operation finishes rather than
  living in a heap page until the next allocation,
- decrypted values are **never logged**, and error paths are written so they cannot include the plaintext.

**Local development** uses a different implementation of the same trait: an HMAC-SHA256 based seal (encrypt-then-MAC)
with a locally generated key. It exists so that the code can be tested without AWS. It is **disabled at boot** when the
Lambda runtime environment variable is present, so the dev path is not reachable in production — asserted, not assumed.

The encryption layer is a single trait with three implementations: `Kms`, `Local`, `Disabled`. The call site does not
know which one it has, and "disabled" is a legitimate state that turns the feature off rather than failing open.

## Alternatives considered

| Alternative | Rejected because |
|---|---|
| Environment variables for config and secrets | Visible in console, in artefacts, in screenshots; rotation requires a deploy |
| AWS Secrets Manager | A good fit, and the right answer if rotation-without-deploy becomes a requirement; at this size the parameter store plus scoped IAM was simpler and cheaper |
| Application-level AES with the key fetched from SSM | Key material ends up in process memory and therefore in heap dumps and crash reports; KMS keeps it out of the process |
| Sign instead of encrypt the sealed value | A signature proves authenticity but the plaintext is still stored server-side; the requirement was that it is not |
| Encrypt with KMS but no encryption context | Nothing binds ciphertext to purpose; a value sealed for one flow can be replayed into another |
| Store `Local` and `Kms` behind runtime branches at the call site | The dev path would ship to production in the code path; a trait plus a boot-time assertion makes it unrepresentable |

## Consequences

**Good**

- No plaintext credential exists in the deployment configuration, so a leaked build artefact is not a leaked secret.
- Ciphertext is **non-transplantable**: changing the purpose in the context makes decryption fail, which turns a class
  of "confused deputy" bug into a signature error.
- Rotation of a credential is a parameter-store change, not a redeploy.
- The zeroizing buffer means the plaintext does not outlive the operation by more than an allocation.

**Bad / accepted costs**

- **Cold-start latency and cost.** Every cold start pays one SSM round trip, and every encrypt/decrypt pays a KMS call.
  This is the honest reason cold start matters here (see ADR 0001); a cache would help and adds invalidation problems.
- **A KMS outage is a feature outage** for anything touching sealed values. There is no local fallback in production, by
  design — a fallback would be a second, weaker encryption scheme reachable by an attacker.
- **The local implementation is a liability if it ever runs in production.** It is a hand-written construction, not an
  audited one. It is fenced by a boot assertion and documented as test-only, but "test-only crypto that ships" is a
  known category of CVE, and it is only acceptable because it cannot be selected at runtime under Lambda.
- **Encryption context is not secret.** It is visible in CloudTrail. It is a binding, not a key.

## What I would do differently

Move to **AWS KMS + a per-tenant data key** (envelope encryption) so that decrypts do not each cost a KMS round trip, and
add an explicit key-rotation procedure with a version prefix in the ciphertext. As written, rotating the master key means
re-encrypting everything, which is fine at this data volume and would not be at ten times it.
