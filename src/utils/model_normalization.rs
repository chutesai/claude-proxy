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
