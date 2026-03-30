use super::*;
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

unsafe fn require_byte_vec(value: i64, label: &str) -> Vec<u8> {
    if let Some(bytes) = crate::binary::bytes_from_int_array(value) {
        return bytes;
    }

    rt_throw_runtime_error(&format!("RuntimeError: {} must be a byte array", label));
}

fn require_non_negative(value: i64, label: &str) -> usize {
    if value < 0 {
        unsafe {
            rt_throw_runtime_error(&format!("RuntimeError: {} must be non-negative", label));
        }
    }

    value as usize
}

fn require_iterations(value: i64) -> u32 {
    if value <= 0 || value > (u32::MAX as i64) {
        unsafe {
            rt_throw_runtime_error(
                "RuntimeError: crypto.pbkdf2Sha256 iterations must be between 1 and 4294967295",
            );
        }
    }

    value as u32
}

#[no_mangle]
pub unsafe extern "C" fn rt_crypto_sha256(data: i64) -> i64 {
    let input = require_byte_vec(data, "crypto.sha256 input");
    let digest = Sha256::digest(&input);
    crate::binary::int_array_from_bytes(digest.as_slice())
}

#[no_mangle]
pub unsafe extern "C" fn rt_crypto_hmac_sha256(key: i64, data: i64) -> i64 {
    let key_bytes = require_byte_vec(key, "crypto.hmacSha256 key");
    let data_bytes = require_byte_vec(data, "crypto.hmacSha256 data");
    let mut mac = HmacSha256::new_from_slice(&key_bytes)
        .expect("HMAC-SHA-256 accepts keys of any length");
    mac.update(&data_bytes);
    let digest = mac.finalize().into_bytes();
    crate::binary::int_array_from_bytes(digest.as_slice())
}

#[no_mangle]
pub unsafe extern "C" fn rt_crypto_pbkdf2_sha256(
    password: i64,
    salt: i64,
    iterations: i64,
    dk_len: i64,
) -> i64 {
    let password_bytes = require_byte_vec(password, "crypto.pbkdf2Sha256 password");
    let salt_bytes = require_byte_vec(salt, "crypto.pbkdf2Sha256 salt");
    let rounds = require_iterations(iterations);
    let output_len = require_non_negative(dk_len, "crypto.pbkdf2Sha256 dkLen");

    let mut output = vec![0u8; output_len];
    pbkdf2_hmac::<Sha256>(&password_bytes, &salt_bytes, rounds, &mut output);
    crate::binary::int_array_from_bytes(&output)
}

#[no_mangle]
pub unsafe extern "C" fn rt_crypto_random_bytes(len: i64) -> i64 {
    let size = require_non_negative(len, "crypto.randomBytes len");
    let mut output = vec![0u8; size];
    OsRng.fill_bytes(&mut output);
    crate::binary::int_array_from_bytes(&output)
}
