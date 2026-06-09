//! JWT validation inside the guest (§6.3). Decode the three base64url segments,
//! check exp/nbf/aud/iss, then verify the signature (RS256 via `rsa`, ES256 via
//! `p256`) against a JWKS key. A failed JWT -> the content path returns a decoy
//! (never a 404).

use alloc::string::String;
use alloc::vec::Vec;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JwtError {
    Malformed,
    Expired,
    NotYetValid,
    IssuerMismatch,
    AudienceMismatch,
    UnknownKey,
    BadSignature,
    UnsupportedAlg,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Claims {
    pub exp: Option<u64>,
    pub nbf: Option<u64>,
    pub iss: Option<String>,
    pub aud: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JwtParts {
    pub alg: String,
    pub kid: Option<String>,
    pub claims: Claims,
    /// raw bytes that the signature covers: `header_b64 . payload_b64`
    pub signing_input: Vec<u8>,
    /// decoded signature bytes
    pub signature: Vec<u8>,
}

pub struct ClaimPolicy<'a> {
    pub now: u64,
    pub expected_iss: Option<&'a str>,
    pub expected_aud: Option<&'a str>,
}

fn seg(v: &[u8]) -> Result<Vec<u8>, JwtError> {
    URL_SAFE_NO_PAD.decode(v).map_err(|_| JwtError::Malformed)
}

pub fn decode_unverified(jwt: &[u8]) -> Result<JwtParts, JwtError> {
    let mut it = jwt.split(|&b| b == b'.');
    let h = it.next().ok_or(JwtError::Malformed)?;
    let p = it.next().ok_or(JwtError::Malformed)?;
    let s = it.next().ok_or(JwtError::Malformed)?;
    if it.next().is_some() {
        return Err(JwtError::Malformed);
    }
    let header: Value = serde_json::from_slice(&seg(h)?).map_err(|_| JwtError::Malformed)?;
    let payload: Value = serde_json::from_slice(&seg(p)?).map_err(|_| JwtError::Malformed)?;
    let alg = header
        .get("alg")
        .and_then(Value::as_str)
        .ok_or(JwtError::Malformed)?
        .into();
    let kid = header.get("kid").and_then(Value::as_str).map(String::from);
    let claims = Claims {
        exp: payload.get("exp").and_then(Value::as_u64),
        nbf: payload.get("nbf").and_then(Value::as_u64),
        iss: payload.get("iss").and_then(Value::as_str).map(String::from),
        aud: payload.get("aud").and_then(Value::as_str).map(String::from),
    };
    let mut signing_input = Vec::with_capacity(h.len() + 1 + p.len());
    signing_input.extend_from_slice(h);
    signing_input.push(b'.');
    signing_input.extend_from_slice(p);
    Ok(JwtParts {
        alg,
        kid,
        claims,
        signing_input,
        signature: seg(s)?,
    })
}

pub fn check_claims(claims: &Claims, policy: &ClaimPolicy) -> Result<(), JwtError> {
    if let Some(exp) = claims.exp {
        if policy.now >= exp {
            return Err(JwtError::Expired);
        }
    }
    if let Some(nbf) = claims.nbf {
        if policy.now < nbf {
            return Err(JwtError::NotYetValid);
        }
    }
    if let Some(want) = policy.expected_iss {
        if claims.iss.as_deref() != Some(want) {
            return Err(JwtError::IssuerMismatch);
        }
    }
    if let Some(want) = policy.expected_aud {
        if claims.aud.as_deref() != Some(want) {
            return Err(JwtError::AudienceMismatch);
        }
    }
    Ok(())
}

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

#[derive(Debug, Clone)]
pub struct Jwk {
    pub kid: String,
    pub kty: String,
    // RSA
    pub n: Option<String>,
    pub e: Option<String>,
    // EC P-256
    pub x: Option<String>,
    pub y: Option<String>,
}

impl Jwk {
    pub fn rsa(kid: &str, n: &str, e: &str) -> Self {
        Jwk {
            kid: kid.into(),
            kty: "RSA".into(),
            n: Some(n.into()),
            e: Some(e.into()),
            x: None,
            y: None,
        }
    }
    pub fn ec_p256(kid: &str, x: &str, y: &str) -> Self {
        Jwk {
            kid: kid.into(),
            kty: "EC".into(),
            n: None,
            e: None,
            x: Some(x.into()),
            y: Some(y.into()),
        }
    }
}

/// Parse a JWKS JSON document into a list of `Jwk`.
pub fn parse_jwks(json: &[u8]) -> Result<Vec<Jwk>, JwtError> {
    let v: Value = serde_json::from_slice(json).map_err(|_| JwtError::Malformed)?;
    let keys = v
        .get("keys")
        .and_then(Value::as_array)
        .ok_or(JwtError::Malformed)?;
    let mut out = Vec::new();
    for k in keys {
        let kid = k.get("kid").and_then(Value::as_str).unwrap_or("").into();
        let kty = k.get("kty").and_then(Value::as_str).unwrap_or("").into();
        out.push(Jwk {
            kid,
            kty,
            n: k.get("n").and_then(Value::as_str).map(String::from),
            e: k.get("e").and_then(Value::as_str).map(String::from),
            x: k.get("x").and_then(Value::as_str).map(String::from),
            y: k.get("y").and_then(Value::as_str).map(String::from),
        });
    }
    Ok(out)
}

pub fn verify_signature(
    alg: &str,
    jwk: &Jwk,
    signing_input: &[u8],
    sig: &[u8],
) -> Result<(), JwtError> {
    match alg {
        "RS256" => verify_rs256(jwk, signing_input, sig),
        "ES256" => verify_es256(jwk, signing_input, sig),
        _ => Err(JwtError::UnsupportedAlg),
    }
}

fn verify_rs256(jwk: &Jwk, signing_input: &[u8], sig: &[u8]) -> Result<(), JwtError> {
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::signature::Verifier;
    use rsa::{BigUint, RsaPublicKey};
    use sha2::Sha256;
    let n = jwk.n.as_ref().ok_or(JwtError::UnknownKey)?;
    let e = jwk.e.as_ref().ok_or(JwtError::UnknownKey)?;
    let n = BigUint::from_bytes_be(&B64.decode(n).map_err(|_| JwtError::Malformed)?);
    let e = BigUint::from_bytes_be(&B64.decode(e).map_err(|_| JwtError::Malformed)?);
    let pk = RsaPublicKey::new(n, e).map_err(|_| JwtError::Malformed)?;
    let vk = VerifyingKey::<Sha256>::new(pk);
    let signature = Signature::try_from(sig).map_err(|_| JwtError::BadSignature)?;
    vk.verify(signing_input, &signature)
        .map_err(|_| JwtError::BadSignature)
}

fn verify_es256(jwk: &Jwk, signing_input: &[u8], sig: &[u8]) -> Result<(), JwtError> {
    use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    use p256::EncodedPoint;
    let x = jwk.x.as_ref().ok_or(JwtError::UnknownKey)?;
    let y = jwk.y.as_ref().ok_or(JwtError::UnknownKey)?;
    let xb = B64.decode(x).map_err(|_| JwtError::Malformed)?;
    let yb = B64.decode(y).map_err(|_| JwtError::Malformed)?;
    let point =
        EncodedPoint::from_affine_coordinates(xb.as_slice().into(), yb.as_slice().into(), false);
    let vk = VerifyingKey::from_encoded_point(&point).map_err(|_| JwtError::Malformed)?;
    let signature = Signature::from_slice(sig).map_err(|_| JwtError::BadSignature)?;
    vk.verify(signing_input, &signature)
        .map_err(|_| JwtError::BadSignature)
}
