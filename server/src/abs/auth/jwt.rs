//! JSON Web Tokens as the original's `libs/jsonwebtoken` makes and checks them: HS256 only, `iat` added to every
//! token, `exp` from an expiry in seconds. The secret is used as its UTF-8 bytes, like the original, so tokens
//! made by Audiobookshelf stay valid in Folio and the other way round.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// Why a token was refused (`JsonWebTokenError` or `TokenExpiredError` in the original).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JwtError {
    /// Not a token, a wrong signature or an algorithm other than HS256.
    Invalid,
    /// A good token whose `exp` has passed.
    Expired,
}

/// HMAC-SHA256 of `data` with `key`.
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(data);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// Seconds since 1970, the unit of `iat` and `exp`.
pub fn now_secs() -> i64 {
    crate::abs::models::now_ms() / 1000
}

/// Signs `payload` (an object) with HS256, adding `iat` and, with `expires_in` seconds, `exp`.
pub fn sign(payload: &Map<String, Value>, secret: &str, expires_in: Option<i64>) -> String {
    let now = now_secs();
    let mut claims = payload.clone();
    claims.entry("iat").or_insert(json!(now));
    if let Some(secs) = expires_in {
        let iat = claims.get("iat").and_then(Value::as_i64).unwrap_or(now);
        claims.insert("exp".into(), json!(iat + secs));
    }
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let body = URL_SAFE_NO_PAD.encode(Value::Object(claims).to_string());
    let signing = format!("{header}.{body}");
    let sig = URL_SAFE_NO_PAD.encode(hmac_sha256(secret.as_bytes(), signing.as_bytes()));
    format!("{signing}.{sig}")
}

/// Checks a token's signature (and, unless `ignore_expiration`, its `exp`) and answers its payload.
pub fn verify(token: &str, secret: &str, ignore_expiration: bool) -> Result<Map<String, Value>, JwtError> {
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(s), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(JwtError::Invalid);
    };
    let header: Value = URL_SAFE_NO_PAD.decode(h).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or(JwtError::Invalid)?;
    if header.get("alg").and_then(Value::as_str) != Some("HS256") {
        return Err(JwtError::Invalid);
    }
    let want = hmac_sha256(secret.as_bytes(), format!("{h}.{p}").as_bytes());
    let got = URL_SAFE_NO_PAD.decode(s).map_err(|_| JwtError::Invalid)?;
    if got.len() != want.len() || got.iter().zip(want.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) != 0 {
        return Err(JwtError::Invalid);
    }
    let payload: Value = URL_SAFE_NO_PAD.decode(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or(JwtError::Invalid)?;
    let Value::Object(claims) = payload else { return Err(JwtError::Invalid) };
    if !ignore_expiration {
        if let Some(exp) = claims.get("exp") {
            let exp = exp.as_f64().ok_or(JwtError::Invalid)?;
            if (now_secs() as f64) >= exp {
                return Err(JwtError::Expired);
            }
        }
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_standard_hs256_token() {
        // made outside Folio (HMAC-SHA256 of Python's standard library) with the secret "secret"
        let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VySWQiOiJhIiwidXNlcm5hbWUiOiJyb290IiwiaWF0IjoxNzAwMDAwMDAwfQ.9ItWAA5vAjIi0LQn0Jg8XKxtthZa8SonfLadketOuAk";
        let claims = verify(token, "secret", true).unwrap();
        assert_eq!(claims["username"], "root");
        assert_eq!(claims["iat"], 1_700_000_000);
        assert_eq!(verify(token, "other", true), Err(JwtError::Invalid));
    }

    #[test]
    fn signs_and_expires() {
        let mut p = Map::new();
        p.insert("userId".into(), json!("u"));
        let t = sign(&p, "k", Some(3600));
        let c = verify(&t, "k", false).unwrap();
        assert_eq!(c["exp"].as_i64().unwrap() - c["iat"].as_i64().unwrap(), 3600);
        let old = sign(&p, "k", Some(-10));
        assert_eq!(verify(&old, "k", false), Err(JwtError::Expired));
        assert!(verify(&old, "k", true).is_ok());
        assert_eq!(verify("a.b", "k", true), Err(JwtError::Invalid));
        assert_eq!(verify("not-a-token", "k", true), Err(JwtError::Invalid));
    }
}
