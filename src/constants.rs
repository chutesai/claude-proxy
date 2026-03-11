// Application-wide constants.
//
// This module centralizes magic numbers and configuration values used throughout
// the application for better maintainability and documentation.

// ============================================================================
// Request Validation Limits
// ============================================================================

/// Maximum number of messages allowed in a single request
/// Matches Anthropic's specification limit of 100,000 messages
pub const MAX_MESSAGES_PER_REQUEST: usize = 100_000;

/// Maximum total content size across all messages (5MB)
/// Prevents payload too large errors and excessive memory usage
pub const MAX_TOTAL_CONTENT_SIZE: usize = 5 * 1024 * 1024;

/// Maximum system prompt size (100KB)
/// System prompts are typically much smaller than message content
pub const MAX_SYSTEM_PROMPT_SIZE: usize = 100 * 1024;

/// Maximum max_tokens parameter value
/// Based on typical model context windows
pub const MAX_TOKENS_LIMIT: u32 = 100_000;

/// Minimum max_tokens parameter value
pub const MIN_TOKENS_LIMIT: u32 = 1;

// ============================================================================
// Token Estimation Constants
// ============================================================================

/// Approximate tokens per image for vision models
/// Based on Claude's image token calculation
pub const TOKENS_PER_IMAGE: usize = 85;

/// Character-to-token ratio for rough estimation (4 chars ≈ 1 token)
/// Used as fallback when tiktoken is unavailable
pub const CHARS_PER_TOKEN: usize = 4;

// ============================================================================
// Circuit Breaker Configuration
// ============================================================================

/// Number of consecutive failures before circuit breaker opens
pub const CIRCUIT_BREAKER_FAILURE_THRESHOLD: u32 = 5;

// ============================================================================
// SSE Streaming Configuration
// ============================================================================

/// SSE channel buffer size for tokio mpsc
/// Balances memory usage with streaming performance
pub const SSE_CHANNEL_BUFFER_SIZE: usize = 64;

// ============================================================================
// Model Configuration
// ============================================================================

/// Default thinking budget tokens for reasoning models
pub const DEFAULT_THINKING_BUDGET_TOKENS: u32 = 10_000;

// ============================================================================
// Helper Functions
// ============================================================================

/// Get price tier emoji based on input/output pricing
/// Used for model list formatting in error messages
pub fn get_price_tier(input_price: Option<f64>, output_price: Option<f64>) -> &'static str {
    // Calculate average price per million tokens
    let avg_price = match (input_price, output_price) {
        (Some(inp), Some(out)) => (inp + out) / 2.0,
        (Some(p), None) | (None, Some(p)) => p,
        (None, None) => return "    ", // No pricing info
    };

    // Price tiers (rough estimates)
    if avg_price < 1.0 {
        "💰" // Very cheap (< $1/M tokens average)
    } else if avg_price < 5.0 {
        "💵" // Affordable ($1-5/M tokens)
    } else if avg_price < 15.0 {
        "💸" // Moderate ($5-15/M tokens)
    } else {
        "💎" // Premium (> $15/M tokens)
    }
}
