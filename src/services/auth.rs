use axum::http::{header::AUTHORIZATION, HeaderMap, HeaderName};

/// Normalize an Authorization header value into a bare API key.
/// Handles case-insensitive "Bearer" prefix per RFC 6750 §2.1.
pub fn normalize_auth_value_to_key(value: &str) -> String {
    let trimmed = value.trim();
    // Case-insensitive: check if starts with "bearer" followed by space or end-of-string
    if trimmed
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bearer"))
    {
        // "Bearer" exactly (no space/token after) → malformed, return empty
        if trimmed.len() == 6 {
            return String::new();
        }
        // "Bearer " or "Bearer <token>" — extract the token part after "Bearer "
        if trimmed.as_bytes()[6] == b' ' {
            let key = trimmed[7..].trim();
            if key.is_empty() {
                return String::new();
            }
            return key.to_string();
        }
    }
    trimmed.to_string()
}

/// Mask sensitive tokens for logs while keeping useful context
pub fn mask_token(token: &str) -> String {
    if token.len() > 12 {
        format!("{}...{}", &token[..6], &token[token.len() - 4..])
    } else if !token.is_empty() {
        "***".to_string()
    } else {
        "<empty>".into()
    }
}

/// Extract client key from headers
pub fn extract_client_key(headers: &HeaderMap) -> Option<String> {
    let x_api_key_header = HeaderName::from_static("x-api-key");
    let raw_x_api_key = headers
        .get(&x_api_key_header)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let raw_authorization = headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    raw_authorization
        .as_deref()
        .map(normalize_auth_value_to_key)
        .filter(|s| !s.is_empty())
        .or(raw_x_api_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    // ============================================================================
    // normalize_auth_value_to_key tests
    // ============================================================================

    #[test]
    fn test_normalize_auth_with_bearer_prefix() {
        let result = normalize_auth_value_to_key("Bearer sk-1234567890");
        assert_eq!(result, "sk-1234567890");
    }

    #[test]
    fn test_normalize_auth_with_bearer_and_extra_spaces() {
        let result = normalize_auth_value_to_key("Bearer   sk-1234567890  ");
        assert_eq!(result, "sk-1234567890");
    }

    #[test]
    fn test_normalize_auth_without_bearer() {
        let result = normalize_auth_value_to_key("sk-1234567890");
        assert_eq!(result, "sk-1234567890");
    }

    #[test]
    fn test_normalize_auth_with_leading_spaces() {
        let result = normalize_auth_value_to_key("   sk-1234567890");
        assert_eq!(result, "sk-1234567890");
    }

    #[test]
    fn test_normalize_auth_empty() {
        let result = normalize_auth_value_to_key("");
        assert_eq!(result, "");
    }

    #[test]
    fn test_normalize_auth_only_bearer_with_space() {
        // "Bearer " with nothing after → empty key
        let result = normalize_auth_value_to_key("Bearer ");
        assert_eq!(result, "");
    }

    #[test]
    fn test_normalize_auth_bearer_only_word() {
        // "Bearer" without trailing space — malformed auth, return empty
        let result = normalize_auth_value_to_key("Bearer");
        assert_eq!(result, "");
    }

    #[test]
    fn test_normalize_auth_lowercase_bearer() {
        // Case-insensitive Bearer prefix per RFC 6750
        let result = normalize_auth_value_to_key("bearer sk-1234567890");
        assert_eq!(result, "sk-1234567890");
    }

    // ============================================================================
    // mask_token tests
    // ============================================================================

    #[test]
    fn test_mask_token_long() {
        let result = mask_token("sk-ant-1234567890abcdefghijklmnop");
        assert_eq!(result, "sk-ant...mnop");
    }

    #[test]
    fn test_mask_token_exactly_13_chars() {
        // Function masks when > 12 chars (not >= 12)
        let result = mask_token("1234567890123");
        assert_eq!(result, "123456...0123");
    }

    #[test]
    fn test_mask_token_short() {
        let result = mask_token("short");
        assert_eq!(result, "***");
    }

    #[test]
    fn test_mask_token_very_short() {
        let result = mask_token("ab");
        assert_eq!(result, "***");
    }

    #[test]
    fn test_mask_token_empty() {
        let result = mask_token("");
        assert_eq!(result, "<empty>");
    }

    #[test]
    fn test_mask_token_anthropic_format() {
        let result = mask_token("sk-ant-api03-1234567890abcdefghijklmnopqrstuvwxyz");
        assert!(result.starts_with("sk-ant"));
        assert!(result.ends_with("wxyz"));
        assert!(result.contains("..."));
    }

    #[test]
    fn test_mask_token_openai_format() {
        let result = mask_token("sk-proj-1234567890abcdefghijklmnop");
        assert!(result.starts_with("sk-pro"));
        assert!(result.ends_with("mnop"));
    }

    // ============================================================================
    // extract_client_key tests
    // ============================================================================

    #[test]
    fn test_extract_client_key_from_authorization() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer sk-test-123"),
        );

        let result = extract_client_key(&headers);
        assert_eq!(result, Some("sk-test-123".to_string()));
    }

    #[test]
    fn test_extract_client_key_from_x_api_key() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sk-test-456"));

        let result = extract_client_key(&headers);
        assert_eq!(result, Some("sk-test-456".to_string()));
    }

    #[test]
    fn test_extract_client_key_authorization_takes_precedence() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer sk-auth-key"),
        );
        headers.insert("x-api-key", HeaderValue::from_static("sk-x-key"));

        let result = extract_client_key(&headers);
        assert_eq!(result, Some("sk-auth-key".to_string()));
    }

    #[test]
    fn test_extract_client_key_no_headers() {
        let headers = HeaderMap::new();

        let result = extract_client_key(&headers);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_client_key_empty_authorization() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static(""));
        headers.insert("x-api-key", HeaderValue::from_static("sk-fallback"));

        let result = extract_client_key(&headers);
        assert_eq!(result, Some("sk-fallback".to_string()));
    }

    #[test]
    fn test_extract_client_key_whitespace_only() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("   "));
        headers.insert("x-api-key", HeaderValue::from_static("sk-fallback"));

        let result = extract_client_key(&headers);
        assert_eq!(result, Some("sk-fallback".to_string()));
    }

    #[test]
    fn test_extract_client_key_both_empty() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static(""));
        headers.insert("x-api-key", HeaderValue::from_static(""));

        let result = extract_client_key(&headers);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_client_key_strips_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer  sk-with-spaces  "),
        );

        let result = extract_client_key(&headers);
        assert_eq!(result, Some("sk-with-spaces".to_string()));
    }

    #[test]
    fn test_extract_client_key_malformed_bearer_falls_back_to_x_api_key() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer"));
        headers.insert("x-api-key", HeaderValue::from_static("sk-fallback"));

        let result = extract_client_key(&headers);
        assert_eq!(result, Some("sk-fallback".to_string()));
    }

    #[test]
    fn test_extract_client_key_malformed_bearer_without_fallback_returns_none() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer "));

        let result = extract_client_key(&headers);
        assert_eq!(result, None);
    }
}
