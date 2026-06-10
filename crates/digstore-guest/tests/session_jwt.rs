mod mock_host;
use digstore_core::ErrorCode;
use digstore_guest::session::{ensure_session, gated_jwks_fetch};
use mock_host::MockHost;

#[test]
fn jwks_fetch_blocked_without_session() {
    let h = MockHost {
        session_ok: false, // no valid session yet
        ..MockHost::default()
    };
    let res = gated_jwks_fetch(&h, b"https://issuer/jwks.json");
    assert_eq!(
        res,
        Err(ErrorCode::NoSession),
        "jwks must be gated until a session exists"
    );
}

#[test]
fn jwks_fetch_allowed_after_session() {
    let h = MockHost {
        session_ok: true,
        jwks: Ok(br#"{"keys":[]}"#.to_vec()),
        ..MockHost::default()
    };
    let res = gated_jwks_fetch(&h, b"https://issuer/jwks.json");
    assert_eq!(res, Ok(br#"{"keys":[]}"#.to_vec()));
}

#[test]
fn ensure_session_establishes_when_absent() {
    // verify_session false -> ensure_session must call establish_session and succeed.
    struct H;
    impl digstore_guest::host::DigHost for H {
        fn get_public_key(&self) -> digstore_guest::host::HostResult {
            Ok(vec![0; 48])
        }
        fn create_attestation(&self, _c: &[u8]) -> digstore_guest::host::HostResult {
            Ok(vec![0; 176])
        }
        fn establish_session(&self, _c: &[u8]) -> digstore_guest::host::HostResult {
            Ok(vec![7; 16])
        }
        fn verify_session(&self) -> bool {
            false
        }
        fn jwks_fetch(&self, _u: &[u8]) -> digstore_guest::host::HostResult {
            Ok(vec![])
        }
        fn current_time(&self) -> u64 {
            1000
        }
        fn random_bytes(&self, c: u32) -> digstore_guest::host::HostResult {
            Ok(vec![1; c as usize])
        }
    }
    let h = H;
    let challenge = [9u8; 72];
    assert!(ensure_session(&h, &challenge).is_ok());
}

use digstore_guest::jwt::{check_claims, decode_unverified, ClaimPolicy, JwtError, JwtParts};

fn b64url(b: &[u8]) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    URL_SAFE_NO_PAD.encode(b)
}

fn make_jwt(header: &str, payload: &str) -> Vec<u8> {
    let mut s = b64url(header.as_bytes());
    s.push('.');
    s.push_str(&b64url(payload.as_bytes()));
    s.push('.');
    s.push_str(&b64url(b"sig"));
    s.into_bytes()
}

#[test]
fn decodes_three_segments() {
    let jwt = make_jwt(
        r#"{"alg":"RS256","kid":"k1"}"#,
        r#"{"exp":2000,"iss":"acme"}"#,
    );
    let parts: JwtParts = decode_unverified(&jwt).expect("decode");
    assert_eq!(parts.alg, "RS256");
    assert_eq!(parts.kid.as_deref(), Some("k1"));
    assert_eq!(parts.claims.exp, Some(2000));
    assert_eq!(parts.claims.iss.as_deref(), Some("acme"));
}

#[test]
fn rejects_expired() {
    let jwt = make_jwt(
        r#"{"alg":"ES256"}"#,
        r#"{"exp":1000,"nbf":0,"iss":"acme","aud":"dig"}"#,
    );
    let parts = decode_unverified(&jwt).unwrap();
    let policy = ClaimPolicy {
        now: 1500,
        expected_iss: Some("acme"),
        expected_aud: Some("dig"),
    };
    assert_eq!(check_claims(&parts.claims, &policy), Err(JwtError::Expired));
}

#[test]
fn rejects_not_yet_valid_and_bad_aud_iss() {
    let parts = decode_unverified(&make_jwt(
        r#"{"alg":"ES256"}"#,
        r#"{"exp":9999,"nbf":5000,"iss":"acme","aud":"dig"}"#,
    ))
    .unwrap();
    let p = ClaimPolicy {
        now: 100,
        expected_iss: Some("acme"),
        expected_aud: Some("dig"),
    };
    assert_eq!(check_claims(&parts.claims, &p), Err(JwtError::NotYetValid));

    let p2 = ClaimPolicy {
        now: 6000,
        expected_iss: Some("other"),
        expected_aud: Some("dig"),
    };
    assert_eq!(
        check_claims(&parts.claims, &p2),
        Err(JwtError::IssuerMismatch)
    );

    let p3 = ClaimPolicy {
        now: 6000,
        expected_iss: Some("acme"),
        expected_aud: Some("nope"),
    };
    assert_eq!(
        check_claims(&parts.claims, &p3),
        Err(JwtError::AudienceMismatch)
    );
}

#[test]
fn accepts_valid_claims() {
    let parts = decode_unverified(&make_jwt(
        r#"{"alg":"RS256"}"#,
        r#"{"exp":9999,"nbf":0,"iss":"acme","aud":"dig"}"#,
    ))
    .unwrap();
    let p = ClaimPolicy {
        now: 5000,
        expected_iss: Some("acme"),
        expected_aud: Some("dig"),
    };
    assert!(check_claims(&parts.claims, &p).is_ok());
}

use digstore_guest::jwt::{verify_signature, Jwk};

#[test]
fn verifies_es256_against_jwk() {
    // Generate an ES256 keypair, sign a known signing_input, build a JWK, verify.
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use p256::ecdsa::{signature::Signer, Signature, SigningKey, VerifyingKey};

    let sk = SigningKey::from_slice(&[7u8; 32]).unwrap();
    let vk: VerifyingKey = *sk.verifying_key();
    let point = vk.to_encoded_point(false);
    let x = URL_SAFE_NO_PAD.encode(point.x().unwrap());
    let y = URL_SAFE_NO_PAD.encode(point.y().unwrap());
    let jwk = Jwk::ec_p256("k1", &x, &y);

    let signing_input = b"eyJhbGciOiJFUzI1NiJ9.eyJpc3MiOiJhY21lIn0";
    let sig: Signature = sk.sign(signing_input);
    let sig_bytes = sig.to_bytes().to_vec(); // raw r||s, 64 bytes (JWT form)

    assert!(verify_signature("ES256", &jwk, signing_input, &sig_bytes).is_ok());
    // tamper
    let mut bad = sig_bytes.clone();
    bad[0] ^= 0xFF;
    assert!(verify_signature("ES256", &jwk, signing_input, &bad).is_err());
}

#[test]
fn verifies_rs256_against_jwk() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use rsa::pkcs1v15::SigningKey;
    use rsa::signature::{SignatureEncoding, Signer};
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPrivateKey;
    use sha2::Sha256;

    let mut rng = rand_core_seeded(); // deterministic key for test speed
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let signing_key = SigningKey::<Sha256>::new(priv_key.clone());
    let pub_key = priv_key.to_public_key();
    let n = URL_SAFE_NO_PAD.encode(pub_key.n().to_bytes_be());
    let e = URL_SAFE_NO_PAD.encode(pub_key.e().to_bytes_be());
    let jwk = Jwk::rsa("k2", &n, &e);

    let signing_input = b"eyJhbGciOiJSUzI1NiJ9.eyJpc3MiOiJhY21lIn0";
    let sig = signing_key.sign(signing_input).to_bytes().to_vec();
    assert!(verify_signature("RS256", &jwk, signing_input, &sig).is_ok());
    let mut bad = sig.clone();
    bad[0] ^= 0xFF;
    assert!(verify_signature("RS256", &jwk, signing_input, &bad).is_err());
}

// Deterministic RNG for the RSA keygen in the test.
fn rand_core_seeded() -> impl rsa::rand_core::CryptoRngCore {
    use rsa::rand_core::SeedableRng;
    rand_chacha::ChaCha8Rng::from_seed([13u8; 32])
}
