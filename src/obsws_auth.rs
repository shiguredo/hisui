use base64::Engine as _;

use crate::obsws_protocol::AUTH_RANDOM_BYTE_LEN;

#[derive(Debug, Clone)]
pub struct ObswsAuthentication {
    pub salt: String,
    pub challenge: String,
    pub expected_response: String,
}

impl ObswsAuthentication {
    pub fn new(password: &str) -> crate::Result<Self> {
        let salt = generate_random_base64(AUTH_RANDOM_BYTE_LEN)?;
        let challenge = generate_random_base64(AUTH_RANDOM_BYTE_LEN)?;
        let expected_response = build_authentication_response(password, &salt, &challenge);
        Ok(Self {
            salt,
            challenge,
            expected_response,
        })
    }

    pub fn is_valid_response(&self, response: Option<&str>) -> bool {
        let Some(response) = response else {
            return false;
        };
        aws_lc_rs::constant_time::verify_slices_are_equal(
            response.as_bytes(),
            self.expected_response.as_bytes(),
        )
        .is_ok()
    }
}

fn generate_random_base64(len: usize) -> crate::Result<String> {
    let mut bytes = vec![0_u8; len];
    aws_lc_rs::rand::fill(&mut bytes)
        .map_err(|_| crate::Error::new("failed to generate random bytes"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

pub fn build_authentication_response(password: &str, salt: &str, challenge: &str) -> String {
    let secret_hash = aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA256,
        format!("{password}{salt}").as_bytes(),
    );
    let secret = base64::engine::general_purpose::STANDARD.encode(secret_hash.as_ref());
    let response_hash = aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA256,
        format!("{secret}{challenge}").as_bytes(),
    );
    base64::engine::general_purpose::STANDARD.encode(response_hash.as_ref())
}
