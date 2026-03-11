use serde_json::Value;

/// Format backend error into user-friendly structured message
pub fn format_backend_error(error_msg: &str, raw_json: &str) -> String {
    // Try to extract model name from context if available
    let model_name = if let Ok(val) = serde_json::from_str::<Value>(raw_json) {
        val.get("model").and_then(|m| m.as_str()).map(String::from)
    } else {
        None
    };

    let mut formatted = String::from("⚠️ Backend Error\n\n");

    if let Some(model) = model_name {
        formatted.push_str(&format!("Model: {}\n", model));
    }

    formatted.push_str(&format!("Error: {}\n\n", error_msg));

    // Add specific suggestions based on error type
    if error_msg.contains("token") && error_msg.contains("exceed") {
        if let Some(requested) = error_msg
            .split("total of ")
            .nth(1)
            .and_then(|s| s.split(" tokens").next())
        {
            formatted.push_str(&format!("Requested: {} tokens\n", requested));
        }
        if let Some(limit) = error_msg
            .split("maximum context length of ")
            .nth(1)
            .and_then(|s| s.split(" tokens").next())
        {
            formatted.push_str(&format!("Limit: {} tokens\n\n", limit));
        }
        formatted.push_str("💡 Suggestions:\n");
        formatted.push_str("• Reduce message history\n");
        formatted.push_str("• Use a model with larger context\n");
        formatted.push_str("• Decrease max_tokens parameter\n");
    } else if error_msg.contains("rate limit") {
        formatted.push_str("💡 Suggestions:\n");
        formatted.push_str("• Wait a moment before retrying\n");
        formatted.push_str("• Check your API quota\n");
    } else if error_msg.contains("insufficient") || error_msg.contains("quota") {
        formatted.push_str("💡 Suggestions:\n");
        formatted.push_str("• Check your account balance\n");
        formatted.push_str("• Verify API key permissions\n");
    }

    formatted
}

/// Build markdown content for synthetic 404 response listing available models
pub fn build_model_list_content(
    requested_model: &str,
    models: &[crate::models::ModelInfo],
) -> String {
    let mut content = format!(
        "❌ Model `{}` not found.\n\n## 📋 Available Models ({} total)\n\n",
        requested_model,
        models.len()
    );

    let mut reasoning_models: Vec<&crate::models::ModelInfo> = vec![];
    let mut standard_models: Vec<&crate::models::ModelInfo> = vec![];

    for model in models {
        let has_reasoning = model
            .supported_features
            .iter()
            .any(|f| f.to_lowercase().contains("reasoning"));

        if has_reasoning {
            reasoning_models.push(model);
        } else {
            standard_models.push(model);
        }
    }

    let sort_models =
        |a: &&crate::models::ModelInfo, b: &&crate::models::ModelInfo| -> std::cmp::Ordering {
            let a_parts: Vec<&str> = a.id.split('/').collect();
            let b_parts: Vec<&str> = b.id.split('/').collect();

            let first_cmp = a_parts
                .first()
                .unwrap_or(&"")
                .to_lowercase()
                .cmp(&b_parts.first().unwrap_or(&"").to_lowercase());

            if first_cmp != std::cmp::Ordering::Equal {
                return first_cmp;
            }

            b_parts
                .get(1)
                .unwrap_or(&"")
                .to_lowercase()
                .cmp(&a_parts.get(1).unwrap_or(&"").to_lowercase())
        };

    reasoning_models.sort_by(sort_models);
    standard_models.sort_by(sort_models);

    let format_two_columns = |models: &[&crate::models::ModelInfo]| -> String {
        let mut result = String::new();
        let half = models.len().div_ceil(2);
        for i in 0..half {
            if let Some(&left_model) = models.get(i) {
                let left_price = crate::constants::get_price_tier(
                    left_model.input_price_usd,
                    left_model.output_price_usd,
                );
                let left_formatted = format!("{:4} {}", left_price, left_model.id);
                if let Some(&right_model) = models.get(i + half) {
                    let right_price = crate::constants::get_price_tier(
                        right_model.input_price_usd,
                        right_model.output_price_usd,
                    );
                    let right_formatted = format!("{:4} {}", right_price, right_model.id);
                    result.push_str(&format!("  {:48} {}\n", left_formatted, right_formatted));
                } else {
                    result.push_str(&format!("  {}\n", left_formatted));
                }
            }
        }
        result
    };

    if !reasoning_models.is_empty() {
        content.push_str("### 🧠 REASONING (Extended Thinking)\n\n");
        content.push_str(&format_two_columns(&reasoning_models));
        content.push('\n');
    }
    if !standard_models.is_empty() {
        content.push_str("### ⚡ STANDARD\n\n");
        content.push_str(&format_two_columns(&standard_models));
        content.push('\n');
    }

    content.push_str("---\n\n💡 **To switch models:** Use `/model <model-name>`");
    content
}
