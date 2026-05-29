use crate::models::{App, ModelInfo};
use serde_json::Value;

/// Build `/v1/models` URL from backend chat completions URL.
fn models_url_from_backend_url(backend_url: &str) -> String {
    // best-effort: replace trailing `/v1/chat/completions` with `/v1/models`
    if let Some(idx) = backend_url.rfind("/v1/chat/completions") {
        let mut s = String::with_capacity(backend_url.len());
        s.push_str(&backend_url[..idx]);
        s.push_str("/v1/models");
        s
    } else {
        // fallback: assume same host, standard path
        format!("{}/../models", backend_url.trim_end_matches('/'))
    }
}

/// Refresh the models cache from backend
pub async fn refresh_models_cache(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let models_url = models_url_from_backend_url(&app.backend_url);
    log::info!("🔄 Fetching available models from {}", models_url);

    // Models endpoint is public (no auth required)
    let mut req = app.client.get(&models_url);
    if let Some(host) = &app.backend_host_header {
        req = req.header("host", host);
    }
    let res = req.send().await?;
    let status = res.status();
    if !status.is_success() {
        // Read error body for debugging
        let error_text = res.text().await.unwrap_or_else(|_| "".into());
        log::warn!(
            "❌ Models endpoint returned {} - response: {}",
            status,
            if error_text.len() > 200 {
                &error_text[..200]
            } else {
                &error_text
            }
        );
        return Err(format!("Models endpoint returned {}", status).into());
    }

    let data: Value = res.json().await?;
    let models: Vec<ModelInfo> = data["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m["id"].as_str()?.to_string();
                    let input_price = m["price"]["input"]["usd"]
                        .as_f64()
                        .or_else(|| m["pricing"]["prompt"].as_f64());
                    let output_price = m["price"]["output"]["usd"]
                        .as_f64()
                        .or_else(|| m["pricing"]["completion"].as_f64());
                    let supported_features = m["supported_features"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(ModelInfo {
                        id,
                        input_price_usd: input_price,
                        output_price_usd: output_price,
                        supported_features,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    log::info!("✅ Cached {} models from backend", models.len());
    let mut cache = app.models_cache.write().await;
    *cache = Some(models);
    Ok(())
}

/// Get cached models or fetch if not available
pub async fn get_available_models(app: &App) -> Vec<ModelInfo> {
    {
        let cache = app.models_cache.read().await;
        if let Some(models) = cache.as_ref() {
            return models.clone();
        }
    }
    if let Err(e) = refresh_models_cache(app).await {
        log::warn!("Failed to fetch models: {}", e);
        return vec![];
    }
    let cache = app.models_cache.read().await;
    cache.as_ref().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CircuitBreakerState;
    use axum::{http::StatusCode, routing::get, Json, Router};
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn make_app(backend_url: String, cached_models: Option<Vec<ModelInfo>>) -> App {
        App {
            client: reqwest::Client::new(),
            backend_url,
            backend_host_header: None,
            models_cache: Arc::new(RwLock::new(cached_models)),
            circuit_breaker: Arc::new(RwLock::new(CircuitBreakerState::new(false))),
        }
    }

    async fn spawn_models_server(status: StatusCode, body: Value) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/v1/models",
            get({
                let body = body.clone();
                move || {
                    let body = body.clone();
                    async move { (status, Json(body)) }
                }
            }),
        );

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        format!("http://{addr}/v1/chat/completions")
    }

    #[test]
    fn test_models_url_from_backend_url_replaces_standard_path() {
        assert_eq!(
            models_url_from_backend_url("http://localhost:8000/v1/chat/completions"),
            "http://localhost:8000/v1/models"
        );
    }

    #[test]
    fn test_models_url_from_backend_url_falls_back_for_nonstandard_path() {
        assert_eq!(
            models_url_from_backend_url("http://localhost:8000/custom/chat"),
            "http://localhost:8000/custom/chat/../models"
        );
    }

    #[tokio::test]
    async fn test_refresh_models_cache_parses_models_response() {
        let backend_url = spawn_models_server(
            StatusCode::OK,
            json!({
                "data": [
                    {
                        "id": "zai-org/GLM-4.5-Air",
                        "price": { "input": { "usd": 0.1 }, "output": { "usd": 0.2 } },
                        "supported_features": []
                    },
                    {
                        "id": "deepseek-r1",
                        "pricing": { "prompt": 0.2, "completion": 0.4 },
                        "supported_features": ["thinking", "extended_thinking"]
                    }
                ]
            }),
        )
        .await;
        let app = make_app(backend_url, None);

        refresh_models_cache(&app).await.unwrap();

        let cache = app.models_cache.read().await;
        let models = cache.as_ref().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "zai-org/GLM-4.5-Air");
        assert_eq!(models[0].input_price_usd, Some(0.1));
        assert_eq!(models[1].id, "deepseek-r1");
        assert_eq!(models[1].output_price_usd, Some(0.4));
        assert!(models[1]
            .supported_features
            .contains(&"thinking".to_string()));
    }

    #[tokio::test]
    async fn test_refresh_models_cache_returns_error_on_non_success() {
        let backend_url = spawn_models_server(
            StatusCode::BAD_GATEWAY,
            json!({ "error": "backend unavailable" }),
        )
        .await;
        let app = make_app(backend_url, None);

        let result = refresh_models_cache(&app).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_available_models_returns_cached_models_without_fetch() {
        let cached_models = vec![ModelInfo {
            id: "cached-model".into(),
            input_price_usd: Some(1.0),
            output_price_usd: Some(2.0),
            supported_features: vec![],
        }];
        let app = make_app(
            "http://127.0.0.1:9/v1/chat/completions".into(),
            Some(cached_models),
        );

        let models = get_available_models(&app).await;
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "cached-model");
    }

    #[tokio::test]
    async fn test_get_available_models_fetches_when_cache_is_empty() {
        let backend_url = spawn_models_server(
            StatusCode::OK,
            json!({
                "data": [
                    { "id": "fetched-model", "supported_features": [] }
                ]
            }),
        )
        .await;
        let app = make_app(backend_url, None);

        let models = get_available_models(&app).await;
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "fetched-model");
    }
}
