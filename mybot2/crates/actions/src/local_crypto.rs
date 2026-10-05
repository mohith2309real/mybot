//! Hashing, identifiers and generators. Nothing here decrypts or cracks:
//! hashes are for checksums and comparing files, the password generator is
//! for the human's new accounts.

use base64::Engine;
use hmac::{Hmac, Mac};
use rand::Rng;
use rand::seq::SliceRandom;
use serde_json::{Value, json};
use sha2::Digest;

use crate::{Action, b_or, local, n_or, s};

const C: &str = "Hashing & IDs";
const T: (&str, &str, &str, bool) = ("text", "string", "Input text", true);

pub fn defs() -> Vec<Action> {
    vec![
        local("hash_sha256", C, "SHA-256 hash of text (hex)", &[T], |v| Ok(hex::encode(sha2::Sha256::digest(s(v, "text")?))), (json!({"text": "abc"}), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")),
        local("hash_sha512", C, "SHA-512 hash of text (hex)", &[T], |v| Ok(hex::encode(sha2::Sha512::digest(s(v, "text")?))), (json!({"text": "abc"}), "ddaf35a193617aba")),
        local("hash_sha1", C, "SHA-1 hash of text (hex) — for checksums, not security", &[T], |v| Ok(hex::encode(sha1::Sha1::digest(s(v, "text")?))), (json!({"text": "abc"}), "a9993e364706816aba3e25717850c26c9cd0d89d")),
        local("hash_md5", C, "MD5 hash of text (hex) — for checksums, not security", &[T], |v| Ok(hex::encode(md5::Md5::digest(s(v, "text")?))), (json!({"text": "abc"}), "900150983cd24fb0d6963f7d28e17f72")),
        local("hmac_sha256", C, "HMAC-SHA256 of a message with a key (hex)", &[("message", "string", "Message", true), ("key", "string", "Key", true)],
            |v| { let mut m = Hmac::<sha2::Sha256>::new_from_slice(s(v, "key")?.as_bytes()).map_err(|e| e.to_string())?; m.update(s(v, "message")?.as_bytes()); Ok(hex::encode(m.finalize().into_bytes())) },
            (json!({"message": "The quick brown fox jumps over the lazy dog", "key": "key"}), "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8")),
        local("crc32", C, "CRC-32 checksum of text", &[T],
            |v| { let mut c: u32 = 0xffff_ffff; for b in s(v, "text")?.bytes() { c ^= b as u32; for _ in 0..8 { c = if c & 1 != 0 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 }; } } Ok(format!("{:08x}", !c)) },
            (json!({"text": "123456789"}), "cbf43926")),
        local("uuid_v4", C, "Generate random UUIDs", &[("count", "integer", "How many (default 1)", false)],
            |v| Ok((0..n_or(v, "count", 1.0).clamp(1.0, 100.0) as usize).map(|_| uuid::Uuid::new_v4().to_string()).collect::<Vec<_>>().join("\n")),
            (json!({"count": 1}), "-4")),
        local("uuid_validate", C, "Check whether text is a valid UUID", &[T],
            |v| Ok(match uuid::Uuid::parse_str(s(v, "text")?.trim()) { Ok(u) => format!("valid UUID, version {}", u.get_version_num()), Err(_) => "not a valid UUID".into() }),
            (json!({"text": "67e55044-10b1-426f-9247-bb680e5fe0c8"}), "valid UUID, version 4")),
        local("random_password", C, "Generate a strong random password for a new account", &[("length", "integer", "Length (default 20)", false), ("symbols", "boolean", "Include symbols (default true)", false)],
            |v| {
                let len = n_or(v, "length", 20.0).clamp(8.0, 128.0) as usize;
                let mut sets: Vec<&[u8]> = vec![b"abcdefghijkmnopqrstuvwxyz", b"ABCDEFGHJKLMNPQRSTUVWXYZ", b"23456789"];
                if b_or(v, "symbols", true) { sets.push(b"!@#$%^&*-_=+?"); }
                let mut rng = rand::thread_rng();
                let all: Vec<u8> = sets.concat();
                let mut pw: Vec<u8> = sets.iter().map(|s2| s2[rng.gen_range(0..s2.len())]).collect();
                while pw.len() < len { pw.push(all[rng.gen_range(0..all.len())]); }
                pw.shuffle(&mut rng);
                Ok(String::from_utf8(pw).unwrap_or_default())
            },
            (json!({"length": 12}), "")),
        local("random_string", C, "Random letters and digits", &[("length", "integer", "Length (default 16)", false)],
            |v| { let mut rng = rand::thread_rng(); const A: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"; Ok((0..n_or(v, "length", 16.0).clamp(1.0, 4096.0) as usize).map(|_| A[rng.gen_range(0..A.len())] as char).collect()) },
            (json!({"length": 8}), "")),
        local("random_number", C, "Random integer between min and max (inclusive)", &[("min", "integer", "Minimum", true), ("max", "integer", "Maximum", true)],
            |v| { let (a, b) = (n_or(v, "min", 0.0) as i64, n_or(v, "max", 100.0) as i64); if a > b { return Err("min is larger than max".into()); } Ok(rand::thread_rng().gen_range(a..=b).to_string()) },
            (json!({"min": 7, "max": 7}), "7")),
        local("random_pick", C, "Pick random items from a list", &[("items", "array", "Items", true), ("count", "integer", "How many (default 1)", false)],
            |v| { let mut it = crate::list(v, "items")?; it.shuffle(&mut rand::thread_rng()); Ok(it.iter().take(n_or(v, "count", 1.0) as usize).map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join("\n")) },
            (json!({"items": ["only"]}), "only")),
        local("jwt_decode", C, "Show a JWT's header and claims (does NOT verify the signature)", &[("token", "string", "JWT", true)],
            |v| {
                let parts: Vec<&str> = s(v, "token")?.trim().split('.').collect();
                if parts.len() < 2 { return Err("not a JWT (expected header.payload.signature)".into()); }
                let dec = |p: &str| -> Result<Value, String> { let b = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(p.trim_end_matches('=')).map_err(|e| e.to_string())?; serde_json::from_slice(&b).map_err(|e| e.to_string()) };
                let (h, p) = (dec(parts[0])?, dec(parts[1])?);
                let exp = p["exp"].as_i64().and_then(|e| chrono::DateTime::from_timestamp(e, 0)).map(|d| format!("\nexpires: {d}")).unwrap_or_default();
                Ok(format!("header: {h}\nclaims: {}{exp}\n(signature not verified)", serde_json::to_string_pretty(&p).unwrap_or_default()))
            },
            (json!({"token": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMiLCJleHAiOjE5MDAwMDAwMDB9.x"}), "\"sub\": \"123\"")),
        local("rot13", C, "ROT13 a text (a toy cipher, for puzzles)", &[T],
            |v| Ok(s(v, "text")?.chars().map(|c| match c { 'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char, 'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char, _ => c }).collect()),
            (json!({"text": "Hello"}), "Uryyb")),
        local("password_strength", C, "Estimate a password's strength without storing or sending it", &[("password", "string", "Password to rate", true)],
            |v| {
                let p = s(v, "password")?;
                let mut pool = 0u32; if p.chars().any(|c| c.is_ascii_lowercase()) { pool += 26; } if p.chars().any(|c| c.is_ascii_uppercase()) { pool += 26; } if p.chars().any(|c| c.is_ascii_digit()) { pool += 10; } if p.chars().any(|c| !c.is_ascii_alphanumeric()) { pool += 32; }
                let bits = p.chars().count() as f64 * (pool.max(1) as f64).log2();
                let common = ["password", "123456", "qwerty", "letmein", "admin", "welcome"].iter().any(|w| p.to_lowercase().contains(w));
                let rating = if common { "weak (contains a very common password)" } else if bits < 40.0 { "weak" } else if bits < 60.0 { "fair" } else if bits < 80.0 { "strong" } else { "very strong" };
                Ok(format!("{rating} — about {bits:.0} bits of entropy"))
            },
            (json!({"password": "correct-Horse-battery-9"}), "very strong")),
        local("luhn_check", C, "Check a number's Luhn checksum (IDs, IMEIs)", &[("number", "string", "Digits", true)],
            |v| { let d: Vec<u32> = s(v, "number")?.chars().filter(|c| c.is_ascii_digit()).filter_map(|c| c.to_digit(10)).collect(); if d.len() < 2 { return Err("too short".into()); } let sum: u32 = d.iter().rev().enumerate().map(|(i, &x)| if i % 2 == 1 { let y = x * 2; if y > 9 { y - 9 } else { y } } else { x }).sum(); Ok(if sum % 10 == 0 { "valid checksum".into() } else { "invalid checksum".into() }) },
            (json!({"number": "79927398713"}), "valid checksum")),
    ]
}
