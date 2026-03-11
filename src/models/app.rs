use crate::constants::*;
use log::warn;
use reqwest::Client;
use std::{sync::Arc, time::SystemTime};
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub id: String,
    pub input_price_usd: Option<f64>,
    pub output_price_usd: Option<f64>,
    pub supported_features: Vec<String>,
}

// ---------- App with cached models and circuit breaker ----------

#[derive(Clone)]
pub struct App {
    pub client: Client,
    pub backend_url: String,
    pub models_cache: Arc<RwLock<Option<Vec<ModelInfo>>>>,
    pub circuit_breaker: Arc<RwLock<CircuitBreakerState>>,
}

// ---------- Circuit breaker state ----------

#[derive(Clone, Debug)]
pub struct CircuitBreakerState {
    pub consecutive_failures: u32,
    pub last_failure_time: Option<SystemTime>,
    pub is_open: bool,
    pub enabled: bool,
}

impl CircuitBreakerState {
    pub fn new(enabled: bool) -> Self {
        Self {
            consecutive_failures: 0,
            last_failure_time: None,
            is_open: false,
            enabled,
        }
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.is_open = false;
        self.last_failure_time = None;
    }

    pub fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        self.last_failure_time = Some(SystemTime::now());
        if self.consecutive_failures >= CIRCUIT_BREAKER_FAILURE_THRESHOLD {
            self.is_open = true;
            warn!(
                "🔴 Circuit breaker opened after {} consecutive failures",
                self.consecutive_failures
            );
        }
    }

    pub fn should_allow_request(&mut self) -> bool {
        if !self.enabled {
            return true;
        }
        if !self.is_open {
            return true;
        }
        // Try to recover after 30 seconds
        if let Some(last_fail) = self.last_failure_time {
            if let Ok(elapsed) = SystemTime::now().duration_since(last_fail) {
                if elapsed.as_secs() >= 30 {
                    log::info!("🟡 Circuit breaker attempting half-open state");
                    self.is_open = false;
                    self.consecutive_failures = 0;
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_circuit_breaker_starts_closed() {
        let breaker = CircuitBreakerState::new(true);
        assert!(!breaker.is_open);
        assert_eq!(breaker.consecutive_failures, 0);
        assert!(breaker.enabled);
    }

    #[test]
    fn test_circuit_breaker_record_success_resets_state() {
        let mut breaker = CircuitBreakerState::new(true);
        breaker.consecutive_failures = 3;
        breaker.is_open = true;
        breaker.last_failure_time = Some(SystemTime::now());

        breaker.record_success();

        assert_eq!(breaker.consecutive_failures, 0);
        assert!(!breaker.is_open);
        assert!(breaker.last_failure_time.is_none());
    }

    #[test]
    fn test_circuit_breaker_opens_after_threshold_failures() {
        let mut breaker = CircuitBreakerState::new(true);
        for _ in 0..CIRCUIT_BREAKER_FAILURE_THRESHOLD {
            breaker.record_failure();
        }

        assert!(breaker.is_open);
        assert_eq!(
            breaker.consecutive_failures,
            CIRCUIT_BREAKER_FAILURE_THRESHOLD
        );
    }

    #[test]
    fn test_circuit_breaker_disabled_always_allows_request() {
        let mut breaker = CircuitBreakerState::new(false);
        breaker.is_open = true;
        breaker.consecutive_failures = CIRCUIT_BREAKER_FAILURE_THRESHOLD;

        assert!(breaker.should_allow_request());
    }

    #[test]
    fn test_circuit_breaker_blocks_request_while_open_before_timeout() {
        let mut breaker = CircuitBreakerState::new(true);
        breaker.is_open = true;
        breaker.consecutive_failures = CIRCUIT_BREAKER_FAILURE_THRESHOLD;
        breaker.last_failure_time = Some(SystemTime::now());

        assert!(!breaker.should_allow_request());
    }

    #[test]
    fn test_circuit_breaker_recovers_after_timeout() {
        let mut breaker = CircuitBreakerState::new(true);
        breaker.is_open = true;
        breaker.consecutive_failures = CIRCUIT_BREAKER_FAILURE_THRESHOLD;
        breaker.last_failure_time = Some(SystemTime::now() - Duration::from_secs(31));

        assert!(breaker.should_allow_request());
        assert!(!breaker.is_open);
        assert_eq!(breaker.consecutive_failures, 0);
    }
}
