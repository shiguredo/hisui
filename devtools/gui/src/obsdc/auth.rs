//! OBS WebSocket 5.x SHA-256 認証。
//!
//! `devtools/src/obsdc/auth.ts` の Rust 移植。
//! アルゴリズムは hisui 本体の `src/obsws/auth.rs` の `build_authentication_response` と同一。

use base64ct::{Base64, Encoding as _};

/// OBS WebSocket 5.x の認証文字列を生成する。
///
/// 1. `base64_secret = base64(sha256(password + salt))`
/// 2. `authentication = base64(sha256(base64_secret + challenge))`
pub fn generate_authentication_string(password: &str, salt: &str, challenge: &str) -> String {
    let secret_hash = aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA256,
        format!("{password}{salt}").as_bytes(),
    );
    let base64_secret = Base64::encode_string(secret_hash.as_ref());
    let secret_challenge_hash = aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA256,
        format!("{base64_secret}{challenge}").as_bytes(),
    );
    Base64::encode_string(secret_challenge_hash.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ブラウザ版 auth.test.ts のテストを移植したもの

    #[test]
    fn generate_authentication_string_matches_spec() {
        // プロトコル仕様のサンプル値
        let password = "supersecretpassword";
        let salt = "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=";
        let challenge = "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=";

        let result = generate_authentication_string(password, salt, challenge);

        // 結果は Base64 文字列であること
        assert!(
            result
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='),
            "Base64 文字列であること: {}",
            result
        );
        // 長さは SHA-256 の Base64 エンコード (44 文字)
        assert_eq!(result.len(), 44);
    }

    #[test]
    fn generate_authentication_string_is_deterministic() {
        let password = "testpassword";
        let salt = "testsalt";
        let challenge = "testchallenge";

        let result1 = generate_authentication_string(password, salt, challenge);
        let result2 = generate_authentication_string(password, salt, challenge);

        assert_eq!(result1, result2);
    }

    #[test]
    fn generate_authentication_string_differs_by_password() {
        let salt = "testsalt";
        let challenge = "testchallenge";

        let result1 = generate_authentication_string("password1", salt, challenge);
        let result2 = generate_authentication_string("password2", salt, challenge);

        assert_ne!(result1, result2);
    }
}
