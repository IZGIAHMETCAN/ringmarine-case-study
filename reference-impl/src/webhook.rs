//! Constant-time verification of signed webhook requests, with a replay window.
//!
//! The ordering in [`Verifier::verify`] is the whole point:
//!
//! 1. reject a missing signature (never treat it as optional),
//! 2. reject a timestamp outside the tolerance window (before doing cryptographic work),
//! 3. reject malformed hex instead of comparing a mangled buffer,
//! 4. compare with [`hmac::Mac::verify_slice`], which is constant time,
//! 5. only then is the body allowed to be deserialised by the caller.
//!
//! The signed message is `"{timestamp}.{body}"`. Binding the timestamp into the signature is what makes the replay
//! window meaningful: an attacker cannot lift a valid signature and pair it with a fresh timestamp.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Domain separator, so a signature produced for this purpose cannot be replayed into another.
const CONTEXT: &[u8] = b"webhook.v1";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignatureError {
    #[error("signature header is missing")]
    MissingSignature,
    #[error("signature is not valid hex")]
    MalformedSignature,
    #[error("signature does not match the payload")]
    InvalidSignature,
    #[error("timestamp {timestamp} is outside the tolerance window around {now}")]
    TimestampOutsideTolerance { timestamp: i64, now: i64 },
    #[error("signing secret must not be empty")]
    EmptySecret,
}

/// An inbound request, as received. Everything is untrusted until [`Verifier::verify`] succeeds.
#[derive(Debug)]
pub struct SignedRequest<'a> {
    pub body: &'a [u8],
    /// The raw hex signature header. `Option` because "absent" and "invalid" are different failures and should be
    /// reported differently.
    pub signature_hex: Option<&'a str>,
    /// Unix seconds, sent alongside the body and covered by the signature.
    pub timestamp: i64,
}

/// Verifies signatures for one shared secret.
///
/// Deliberately does not implement `Debug`: it holds the secret, and a derived implementation would put it into any
/// log line, panic message or error report that formatted the verifier. If you need `Debug`, write it by hand and
/// redact the field.
pub struct Verifier {
    secret: Vec<u8>,
    tolerance_secs: i64,
}

impl Verifier {
    /// `tolerance_secs` is the accepted clock skew in both directions. Pick it small enough that a captured request
    /// is not replayable for long, and large enough to survive normal clock drift between two hosts.
    pub fn new(secret: impl Into<Vec<u8>>, tolerance_secs: i64) -> Result<Self, SignatureError> {
        let secret = secret.into();
        if secret.is_empty() {
            return Err(SignatureError::EmptySecret);
        }
        Ok(Self { secret, tolerance_secs })
    }

    /// Verify a request as of `now` (unix seconds).
    pub fn verify(&self, request: &SignedRequest<'_>, now: i64) -> Result<(), SignatureError> {
        let signature_hex = request.signature_hex.ok_or(SignatureError::MissingSignature)?;

        if (now - request.timestamp).abs() > self.tolerance_secs {
            return Err(SignatureError::TimestampOutsideTolerance {
                timestamp: request.timestamp,
                now,
            });
        }

        let received = hex::decode(signature_hex.trim())
            .map_err(|_| SignatureError::MalformedSignature)?;

        let mut mac = HmacSha256::new_from_slice(&self.secret)
            .map_err(|_| SignatureError::EmptySecret)?;
        mac.update(&signed_message(request.timestamp, request.body));

        // Constant time, and checks the length as part of the comparison.
        mac.verify_slice(&received)
            .map_err(|_| SignatureError::InvalidSignature)
    }
}

fn signed_message(timestamp: i64, body: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(CONTEXT.len() + 32 + body.len());
    message.extend_from_slice(CONTEXT);
    message.push(b'.');
    message.extend_from_slice(timestamp.to_string().as_bytes());
    message.push(b'.');
    message.extend_from_slice(body);
    message
}

/// Produce a signature. This is the provider's half of the protocol; it lives here so the tests — and any local
/// integration you build — can generate valid requests without calling the real provider.
pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("secret is non-empty");
    mac.update(&signed_message(timestamp, body));
    hex::encode(mac.finalize().into_bytes())
}
