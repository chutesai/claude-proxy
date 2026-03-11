use crate::models::ModelInfo;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Passthrough model with case-correction from cache
pub async fn normalize_model_name(
    model: &str,
    models_cache: &Arc<RwLock<Option<Vec<ModelInfo>>>>,
) -> String {
    let model_lower = model.to_lowercase();
    let cache = models_cache.read().await;
    if let Some(models) = cache.as_ref() {
        if models.iter().any(|m| m.id == model) {
            return model.to_string();
        }
        if let Some(matched) = models.iter().find(|m| m.id.to_lowercase() == model_lower) {
            log::info!("🔄 Model: {} → {} (case-corrected)", model, matched.id);
            return matched.id.clone();
        }
    }
    model.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_with_models(models: Vec<ModelInfo>) -> Arc<RwLock<Option<Vec<ModelInfo>>>> {
        Arc::new(RwLock::new(Some(models)))
    }

    #[tokio::test]
    async fn test_normalize_model_name_returns_exact_match() {
        let cache = cache_with_models(vec![ModelInfo {
            id: "zai-org/GLM-4.5-Air".into(),
            input_price_usd: None,
            output_price_usd: None,
            supported_features: vec![],
        }]);

        let normalized = normalize_model_name("zai-org/GLM-4.5-Air", &cache).await;
        assert_eq!(normalized, "zai-org/GLM-4.5-Air");
    }

    #[tokio::test]
    async fn test_normalize_model_name_case_corrects_match() {
        let cache = cache_with_models(vec![ModelInfo {
            id: "zai-org/GLM-4.5-Air".into(),
            input_price_usd: None,
            output_price_usd: None,
            supported_features: vec![],
        }]);

        let normalized = normalize_model_name("zai-org/glm-4.5-air", &cache).await;
        assert_eq!(normalized, "zai-org/GLM-4.5-Air");
    }

    #[tokio::test]
    async fn test_normalize_model_name_returns_original_when_not_found() {
        let cache = cache_with_models(vec![ModelInfo {
            id: "deepseek-r1".into(),
            input_price_usd: None,
            output_price_usd: None,
            supported_features: vec![],
        }]);

        let normalized = normalize_model_name("missing-model", &cache).await;
        assert_eq!(normalized, "missing-model");
    }

    #[tokio::test]
    async fn test_normalize_model_name_returns_original_when_cache_empty() {
        let cache = Arc::new(RwLock::new(None));
        let normalized = normalize_model_name("zai-org/GLM-4.5-Air", &cache).await;
        assert_eq!(normalized, "zai-org/GLM-4.5-Air");
    }
}
