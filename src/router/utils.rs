use crate::app_error::ServerError;

/// Generous upper bound for an encoded specifier list; legitimate queries
/// (even ~1000 dependencies) stay well under it.
const MAX_ENCODED_LEN: usize = 64 * 1024;

pub fn decode_base64(part: &str) -> Result<String, ServerError> {
    if part.len() > MAX_ENCODED_LEN {
        return Err(ServerError::InvalidQuery);
    }
    let decoded = base64_simd::STANDARD
        .decode_to_vec(part.as_bytes())
        .map_err(|_e| ServerError::Base64DecodingError())?;
    let val = String::from_utf8(decoded).map_err(|_e| ServerError::Base64DecodingError())?;
    Ok(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_decodes() {
        // "react@^18.0.0" in standard base64
        assert_eq!(decode_base64("cmVhY3RAXjE4LjAuMA==").unwrap(), "react@^18.0.0");
    }

    #[test]
    fn oversized_input_is_rejected() {
        let oversized = "A".repeat(MAX_ENCODED_LEN + 1);
        assert!(matches!(
            decode_base64(&oversized),
            Err(ServerError::InvalidQuery)
        ));
    }

    #[test]
    fn invalid_base64_is_rejected() {
        assert!(matches!(
            decode_base64("!!!"),
            Err(ServerError::Base64DecodingError())
        ));
    }
}
