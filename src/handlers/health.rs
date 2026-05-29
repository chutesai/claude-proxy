use crate::models::App;
use axum::{extract::State, response::Json};
use serde_json::{json, Value};

/// Health check endpoint
pub async fn health_check(State(app): State<App>) -> Json<Value> {
    let models = crate::services::model_cache::get_available_models(&app).await;
    let circuit_breaker = app.circuit_breaker.read().await;

    let status = if circuit_breaker.is_open {
        "unhealthy"
    } else {
        "healthy"
    };

    Json(json!({
        "status": status,
        "backend_url": app.backend_url,
        "models_cached": models.len(),
        "circuit_breaker": {
            "enabled": circuit_breaker.enabled,
            "is_open": circuit_breaker.is_open,
            "consecutive_failures": circuit_breaker.consecutive_failures
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CircuitBreakerState, ModelInfo};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn test_app(is_open: bool, cached_models: usize) -> App {
        App {
            client: reqwest::Client::new(),
            backend_url: "http://127.0.0.1:8000/v1/chat/completions".into(),
            backend_host_header: None,
            models_cache: Arc::new(RwLock::new(Some(
                (0..cached_models)
                    .map(|idx| ModelInfo {
                        id: format!("model-{idx}"),
                        input_price_usd: None,
                        output_price_usd: None,
                        supported_features: vec![],
                    })
                    .collect(),
            ))),
            circuit_breaker: Arc::new(RwLock::new(CircuitBreakerState {
                consecutive_failures: if is_open { 5 } else { 0 },
                last_failure_time: None,
                is_open,
                enabled: true,
            })),
        }
    }

    #[tokio::test]
    async fn test_health_check_reports_healthy_status() {
        let Json(body) = health_check(State(test_app(false, 2))).await;

        assert_eq!(body["status"], "healthy");
        assert_eq!(body["models_cached"], 2);
        assert_eq!(body["circuit_breaker"]["is_open"], false);
    }

    #[tokio::test]
    async fn test_health_check_reports_unhealthy_status_when_breaker_open() {
        let Json(body) = health_check(State(test_app(true, 1))).await;

        assert_eq!(body["status"], "unhealthy");
        assert_eq!(body["models_cached"], 1);
        assert_eq!(body["circuit_breaker"]["is_open"], true);
        assert_eq!(body["circuit_breaker"]["consecutive_failures"], 5);
    }
}
