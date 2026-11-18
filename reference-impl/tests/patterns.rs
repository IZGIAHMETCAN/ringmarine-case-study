//! The scenarios that motivated the two modules, written as tests.
//!
//! Each test name states the failure it prevents, because that is the part that survives the code being rewritten.

use payment_patterns::idempotency::{Begin, IdempotencyStore, StoredResponse};
use payment_patterns::webhook::{sign, SignatureError, SignedRequest, Verifier};

const SECRET: &[u8] = b"a-shared-secret-that-never-touches-the-repository";
const T0: i64 = 1_800_000_000; // fixed clock: expiry is only testable if time is an input
const TOLERANCE: i64 = 300;

fn ok_response() -> StoredResponse {
    StoredResponse {
        status: 200,
        body: r#"{"credited":true,"balance":"42.00"}"#.to_string(),
    }
}

// ---------------------------------------------------------------- idempotency

#[test]
fn duplicate_delivery_results_in_one_effect() {
    let store = IdempotencyStore::new(3600);

    // First delivery wins the claim and does the work.
    let claim = match store.begin("payment:conv-1", T0) {
        Begin::Start(claim) => claim,
        other => panic!("expected the first delivery to win the claim, got {other:?}"),
    };
    claim.complete(ok_response(), T0);

    // Second delivery — the duplicate webhook, or the browser callback arriving after it.
    match store.begin("payment:conv-1", T0 + 5) {
        Begin::Completed(response) => assert_eq!(response, ok_response()),
        other => panic!("expected the stored response, got {other:?}"),
    }
}

#[test]
fn concurrent_delivery_is_told_in_progress_not_given_a_second_chance() {
    let store = IdempotencyStore::new(3600);

    let first = match store.begin("payment:conv-2", T0) {
        Begin::Start(claim) => claim,
        other => panic!("expected Start, got {other:?}"),
    };

    // The other channel arrives while the first is still working. It must not do the work again, and it must not
    // be told the payment failed either.
    assert!(matches!(
        store.begin("payment:conv-2", T0 + 1),
        Begin::InProgress
    ));

    drop(first);
}

#[test]
fn abandoned_claim_can_be_retried() {
    let store = IdempotencyStore::new(3600);

    // The handler errored out after claiming — dropping the claim must release it, otherwise the payment is stuck
    // in "in progress" forever and the user can never complete it.
    {
        let _claim = match store.begin("payment:conv-3", T0) {
            Begin::Start(claim) => claim,
            other => panic!("expected Start, got {other:?}"),
        };
    }

    let retry = store.begin("payment:conv-3", T0 + 10);
    assert!(
        matches!(retry, Begin::Start(_)),
        "a retry after an abandoned claim must be allowed to proceed"
    );
}

#[test]
fn expired_record_is_treated_as_absent_even_if_still_stored() {
    let store = IdempotencyStore::new(60);

    let claim = match store.begin("payment:conv-4", T0) {
        Begin::Start(claim) => claim,
        other => panic!("expected Start, got {other:?}"),
    };
    claim.complete(ok_response(), T0);

    // Inside the TTL: the stored response is reused.
    assert!(matches!(
        store.begin("payment:conv-4", T0 + 59),
        Begin::Completed(_)
    ));

    // Past the TTL: the record may still physically exist (that is how DynamoDB TTL behaves), but it must be
    // treated as gone — otherwise an abandoned checkout session stays valid indefinitely.
    assert!(matches!(
        store.begin("payment:conv-4", T0 + 61),
        Begin::Start(_)
    ));
}

#[test]
fn different_keys_do_not_block_each_other() {
    let store = IdempotencyStore::new(3600);

    let a = match store.begin("payment:conv-5", T0) {
        Begin::Start(claim) => claim,
        other => panic!("expected Start, got {other:?}"),
    };
    let b = store.begin("payment:conv-6", T0);
    assert!(
        matches!(b, Begin::Start(_)),
        "claims are scoped per key, not globally"
    );

    drop(a);
}

// -------------------------------------------------------------------- webhook

fn verifier() -> Verifier {
    Verifier::new(SECRET.to_vec(), TOLERANCE).expect("non-empty secret")
}

fn valid_request<'a>(body: &'a [u8], signature: &'a str) -> SignedRequest<'a> {
    SignedRequest {
        body,
        signature_hex: Some(signature),
        timestamp: T0,
    }
}

#[test]
fn valid_signature_is_accepted() {
    let body = br#"{"status":"success","paymentId":"p-1"}"#;
    let signature = sign(SECRET, T0, body);

    verifier()
        .verify(&valid_request(body, &signature), T0)
        .expect("a correctly signed request must verify");
}

#[test]
fn missing_signature_is_rejected_rather_than_assumed_trustworthy() {
    let body = br#"{"status":"success"}"#;
    let error = verifier()
        .verify(
            &SignedRequest {
                body,
                signature_hex: None,
                timestamp: T0,
            },
            T0,
        )
        .unwrap_err();

    assert_eq!(error, SignatureError::MissingSignature);
}

#[test]
fn tampered_body_is_rejected() {
    let body = br#"{"status":"success","amount":"10.00"}"#;
    let signature = sign(SECRET, T0, body);

    let tampered = br#"{"status":"success","amount":"1000.00"}"#;
    let error = verifier()
        .verify(&valid_request(tampered, &signature), T0)
        .unwrap_err();

    assert_eq!(error, SignatureError::InvalidSignature);
}

#[test]
fn signature_from_another_secret_is_rejected() {
    let body = br#"{"status":"success"}"#;
    let wrong = sign(b"not-the-shared-secret", T0, body);
    let error = verifier()
        .verify(&valid_request(body, &wrong), T0)
        .unwrap_err();

    assert_eq!(error, SignatureError::InvalidSignature);
}

#[test]
fn malformed_hex_is_rejected_as_such() {
    let body = br#"{"status":"success"}"#;
    let error = verifier()
        .verify(&valid_request(body, "zzzz-not-hex"), T0)
        .unwrap_err();

    assert_eq!(error, SignatureError::MalformedSignature);
}

#[test]
fn replay_outside_the_tolerance_window_is_rejected() {
    let body = br#"{"status":"success"}"#;
    // A signature captured an hour ago is still cryptographically valid; it must still be refused.
    let old = T0 - 3600;
    let signature = sign(SECRET, old, body);

    let error = verifier()
        .verify(
            &SignedRequest {
                body,
                signature_hex: Some(&signature),
                timestamp: old,
            },
            T0,
        )
        .unwrap_err();

    assert_eq!(
        error,
        SignatureError::TimestampOutsideTolerance {
            timestamp: old,
            now: T0
        }
    );
}

#[test]
fn empty_secret_is_refused_at_construction() {
    // `unwrap_err` is not used here on purpose: `Verifier` deliberately does not implement `Debug`, because it holds
    // the shared secret and a derived `Debug` would print it into whatever log a panic lands in.
    let error = match Verifier::new(Vec::new(), TOLERANCE) {
        Ok(_) => panic!("an empty secret must be refused"),
        Err(error) => error,
    };
    assert_eq!(error, SignatureError::EmptySecret);
}
