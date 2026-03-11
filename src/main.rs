use axum::{
    routing::{get, post},
    Router,
};
use log::info;
use std::{env, sync::Arc, time::Duration};
use tokio::sync::RwLock;

// Import our modules
mod constants;
mod handlers;
mod models;
mod services;
mod utils;

use models::{App, CircuitBreakerState};
use services::model_cache::refresh_models_cache;

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimeConfig {
    backend_url: String,
    backend_timeout_secs: u64,
    circuit_breaker_enabled: bool,
    host_port: u16,
}

impl RuntimeConfig {
    fn from_env() -> Self {
        Self::from_env_with(|key| env::var(key).ok())
    }

    fn from_env_with(mut get: impl FnMut(&str) -> Option<String>) -> Self {
        Self {
            backend_url: get("BACKEND_URL")
                .unwrap_or_else(|| "http://127.0.0.1:8000/v1/chat/completions".into()),
            backend_timeout_secs: get("BACKEND_TIMEOUT_SECS")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(600),
            circuit_breaker_enabled: get("ENABLE_CIRCUIT_BREAKER")
                .and_then(|value| value.parse::<bool>().ok())
                .unwrap_or(false),
            host_port: get("HOST_PORT")
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(8080),
        }
    }
}

fn build_app(config: &RuntimeConfig) -> App {
    let models_cache = Arc::new(RwLock::new(None));
    let circuit_breaker = Arc::new(RwLock::new(CircuitBreakerState::new(
        config.circuit_breaker_enabled,
    )));

    App {
        client: reqwest::Client::builder()
            .pool_max_idle_per_host(1024)
            .tcp_keepalive(Some(Duration::from_secs(60)))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(config.backend_timeout_secs))
            .build()
            .unwrap(),
        backend_url: config.backend_url.clone(),
        models_cache,
        circuit_breaker,
    }
}

fn build_router(app: App) -> Router {
    Router::new()
        .route("/health", get(handlers::health_check))
        .route("/v1/messages", post(handlers::messages))
        .route("/v1/messages/count_tokens", post(handlers::count_tokens))
        .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(app)
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config = RuntimeConfig::from_env();

    info!("🚀 Claude-to-OpenAI Proxy starting...");
    info!("   Backend URL: {}", config.backend_url);
    info!("   Backend Timeout: {}s", config.backend_timeout_secs);
    info!(
        "   Circuit Breaker: {}",
        if config.circuit_breaker_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!("   Mode: Passthrough with case-correction");

    let app = build_app(&config);

    // Initial model cache load (blocking - must complete before accepting requests)
    info!("🔄 Loading initial model cache...");
    if let Err(e) = refresh_models_cache(&app).await {
        log::warn!(
            "⚠️  Failed to load initial model cache: {}. Continuing anyway.",
            e
        );
    }

    // Background model cache refresh (every 60s) with graceful shutdown
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
    let cache_task = {
        let app_clone = app.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = refresh_models_cache(&app_clone).await {
                    log::warn!("Failed to refresh models cache: {}", e);
                }

                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {
                        // Continue loop
                    }
                    _ = shutdown_rx.recv() => {
                        info!("🛑 Model cache refresh task shutting down gracefully");
                        break;
                    }
                }
            }
        })
    };

    let router = build_router(app.clone());

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.host_port))
        .await
        .unwrap();
    info!("   Listening on: 0.0.0.0:{}", config.host_port);

    // Graceful shutdown: use axum's built-in mechanism
    let server = axum::serve(listener, router).with_graceful_shutdown(async {
        tokio::signal::ctrl_c().await.ok();
        info!("🛑 Received shutdown signal, draining connections...");
    });

    // Run server (this will complete when graceful shutdown finishes)
    if let Err(e) = server.await {
        log::error!("Server error: {}", e);
    }

    // After server is shut down, clean up background tasks
    info!("🧹 Cleaning up background tasks...");
    let _ = shutdown_tx.send(()).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), cache_task).await;
    info!("✅ Shutdown complete");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use serde_json::{json, Value};
    use tower::ServiceExt;

    #[test]
    fn test_runtime_config_uses_defaults() {
        let config = RuntimeConfig::from_env_with(|_| None);

        assert_eq!(
            config,
            RuntimeConfig {
                backend_url: "http://127.0.0.1:8000/v1/chat/completions".into(),
                backend_timeout_secs: 600,
                circuit_breaker_enabled: false,
                host_port: 8080,
            }
        );
    }

    #[test]
    fn test_runtime_config_respects_overrides_and_fallbacks() {
        let config = RuntimeConfig::from_env_with(|key| match key {
            "BACKEND_URL" => Some("https://example.com/v1/chat/completions".into()),
            "BACKEND_TIMEOUT_SECS" => Some("45".into()),
            "ENABLE_CIRCUIT_BREAKER" => Some("true".into()),
            "HOST_PORT" => Some("9090".into()),
            _ => None,
        });

        assert_eq!(
            config,
            RuntimeConfig {
                backend_url: "https://example.com/v1/chat/completions".into(),
                backend_timeout_secs: 45,
                circuit_breaker_enabled: true,
                host_port: 9090,
            }
        );

        let fallback_config = RuntimeConfig::from_env_with(|key| match key {
            "BACKEND_TIMEOUT_SECS" => Some("not-a-number".into()),
            "ENABLE_CIRCUIT_BREAKER" => Some("not-a-bool".into()),
            "HOST_PORT" => Some("70000".into()),
            _ => None,
        });

        assert_eq!(fallback_config.backend_timeout_secs, 600);
        assert!(!fallback_config.circuit_breaker_enabled);
        assert_eq!(fallback_config.host_port, 8080);
    }

    #[tokio::test]
    async fn test_build_app_uses_runtime_config() {
        let config = RuntimeConfig {
            backend_url: "http://127.0.0.1:9999/v1/chat/completions".into(),
            backend_timeout_secs: 123,
            circuit_breaker_enabled: true,
            host_port: 8080,
        };

        let app = build_app(&config);

        assert_eq!(app.backend_url, config.backend_url);
        assert!(app.models_cache.read().await.is_none());
        assert!(app.circuit_breaker.read().await.enabled);
    }

    #[tokio::test]
    async fn test_build_router_serves_health_and_count_tokens_routes() {
        let app = build_app(&RuntimeConfig::from_env_with(|_| None));
        *app.models_cache.write().await = Some(vec![]);
        let router = build_router(app);

        let health_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health_response.status(), StatusCode::OK);

        let count_tokens_response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages/count_tokens")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "model": "zai-org/GLM-4.5-Air",
                            "messages": [{ "role": "user", "content": "hello" }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(count_tokens_response.status(), StatusCode::OK);
        let body = to_bytes(count_tokens_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert!(parsed["input_tokens"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_build_router_routes_messages_endpoint() {
        let app = build_app(&RuntimeConfig::from_env_with(|_| None));
        let router = build_router(app);

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "model": "zai-org/GLM-4.5-Air",
                            "messages": [{ "role": "user", "content": "hi" }],
                            "stream": false
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
