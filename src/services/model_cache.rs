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
    let res = app.client.get(&models_url).send().await?;
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
