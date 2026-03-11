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
