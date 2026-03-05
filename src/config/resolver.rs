//! Model string resolution for Villalobos
//!
//! This module provides functionality to parse model ID strings and resolve
//! user model input to concrete model identifiers. It supports:
//!
//! - Parsing model strings into family and version components
//! - Resolving family-only input (e.g., "gpt") to the highest available version
//! - Exact matching for versioned input (e.g., "sonnet4.5")
//!
//! # Examples
//!
//! ```
//! use villalobos::config::resolver::{parse_model_string, resolve_model};
//!
//! // Parse a model string
//! let (family, version) = parse_model_string("sonnet4.5");
//! assert_eq!(family, "sonnet");
//! assert_eq!(version, Some(4.5));
//!
//! // Parse a family-only string
//! let (family, version) = parse_model_string("opus");
//! assert_eq!(family, "opus");
//! assert_eq!(version, None);
//! ```

use anyhow::{anyhow, Result};

use crate::models::AvailableModel;

/// Parses a model ID string into its family name and optional version number.
///
/// The function splits the model string at the boundary where letters meet numbers,
/// extracting the alphabetic family prefix and the numeric version suffix.
///
/// # Arguments
///
/// * `s` - A model ID string (e.g., "sonnet4.5", "gpt5.1", "opus")
///
/// # Returns
///
/// A tuple of (family, Option<version>) where:
/// - `family` is the alphabetic prefix (e.g., "sonnet", "gpt", "opus")
/// - `version` is `Some(f32)` if a version number is present, `None` otherwise
///
/// # Examples
///
/// ```
/// let (family, version) = parse_model_string("sonnet4.5");
/// assert_eq!(family, "sonnet");
/// assert_eq!(version, Some(4.5));
///
/// let (family, version) = parse_model_string("opus");
/// assert_eq!(family, "opus");
/// assert_eq!(version, None);
/// ```
pub fn parse_model_string(s: &str) -> (String, Option<f32>) {
    let s = s.trim().to_lowercase();

    // Find the index where letters end and numbers begin
    let split_index = s
        .char_indices()
        .find(|(_, c)| c.is_ascii_digit())
        .map(|(i, _)| i);

    match split_index {
        Some(idx) if idx > 0 => {
            let family = s[..idx].to_string();
            let version_str = &s[idx..];
            let version = version_str.parse::<f32>().ok();
            (family, version)
        }
        _ => {
            // No numeric portion found, or string starts with a digit
            (s, None)
        }
    }
}

/// Resolves user model input to a concrete model ID string.
///
/// This function takes a user's model input and a list of available models,
/// and returns the model ID string that should be used. The resolution logic is:
///
/// 1. If the input contains a version number (e.g., "sonnet4"), find an exact match
/// 2. If the input is family-only (e.g., "gpt"), find all models with that prefix
///    and return the one with the highest version number
///
/// # Arguments
///
/// * `user_input` - The user's model input string (e.g., "gpt", "sonnet4.5")
/// * `available_models` - A slice of available models to search
///
/// # Returns
///
/// `Ok(String)` with the resolved model ID, or `Err` if no matching model is found
///
/// # Errors
///
/// Returns an error if:
/// - No model matches the user input
/// - The input contains a version but no exact match exists
///
/// # Examples
///
/// ```
/// // With available models: ["gpt5", "gpt5.1", "sonnet4", "sonnet4.5"]
///
/// // Family-only input returns highest version
/// resolve_model("gpt", &available_models)  // Returns "gpt5.1"
///
/// // Versioned input returns exact match
/// resolve_model("sonnet4", &available_models)  // Returns "sonnet4"
/// ```
pub fn resolve_model(user_input: &str, available_models: &[AvailableModel]) -> Result<String> {
    let (input_family, input_version) = parse_model_string(user_input);

    if input_family.is_empty() {
        return Err(anyhow!("Invalid model input: empty string"));
    }

    // Collect all models that match the family prefix
    let mut matching_models: Vec<(&AvailableModel, Option<f32>)> = available_models
        .iter()
        .filter_map(|model| {
            let model_id_str = model.id.as_str();
            let (model_family, model_version) = parse_model_string(model_id_str);

            if model_family == input_family {
                Some((model, model_version))
            } else {
                None
            }
        })
        .collect();

    if matching_models.is_empty() {
        return Err(anyhow!(
            "No model found matching '{}'. Available families: {:?}",
            user_input,
            get_available_families(available_models)
        ));
    }

    // If user specified a version, look for exact match
    if let Some(requested_version) = input_version {
        for (model, model_version) in &matching_models {
            if let Some(mv) = model_version {
                // Compare versions with small epsilon for floating point comparison
                if (mv - requested_version).abs() < 0.001 {
                    return Ok(model.id.as_str().to_string());
                }
            }
        }
        // No exact match found for the specified version
        return Err(anyhow!(
            "No exact match for '{}'. Available versions for '{}': {:?}",
            user_input,
            input_family,
            matching_models
                .iter()
                .filter_map(|(_, v)| *v)
                .collect::<Vec<_>>()
        ));
    }

    // No version specified - return highest version
    matching_models.sort_by(|a, b| {
        let v_a = a.1.unwrap_or(0.0);
        let v_b = b.1.unwrap_or(0.0);
        v_b.partial_cmp(&v_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(matching_models[0].0.id.as_str().to_string())
}

/// Extracts unique family names from a list of available models.
fn get_available_families(available_models: &[AvailableModel]) -> Vec<String> {
    let mut families: Vec<String> = available_models
        .iter()
        .map(|m| parse_model_string(m.id.as_str()).0)
        .collect();
    families.sort();
    families.dedup();
    families
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ModelId;

    // ========================================================================
    // Helper function to create test models
    // ========================================================================

    fn create_test_model(id: ModelId) -> AvailableModel {
        AvailableModel {
            id,
            name: format!("{:?}", id),
            description: "Test model".to_string(),
        }
    }

    fn create_test_models() -> Vec<AvailableModel> {
        vec![
            create_test_model(ModelId::Haiku4_5),
            create_test_model(ModelId::Opus4_5),
            create_test_model(ModelId::Sonnet4),
            create_test_model(ModelId::Sonnet4_5),
            create_test_model(ModelId::Gpt5),
            create_test_model(ModelId::Gpt5_1),
        ]
    }

    // ========================================================================
    // parse_model_string Tests
    // ========================================================================

    #[test]
    fn test_parse_model_string_with_decimal_version() {
        let (family, version) = parse_model_string("sonnet4.5");
        assert_eq!(family, "sonnet");
        assert_eq!(version, Some(4.5));
    }

    #[test]
    fn test_parse_model_string_with_decimal_version_gpt() {
        let (family, version) = parse_model_string("gpt5.1");
        assert_eq!(family, "gpt");
        assert_eq!(version, Some(5.1));
    }

    #[test]
    fn test_parse_model_string_with_integer_version() {
        let (family, version) = parse_model_string("sonnet4");
        assert_eq!(family, "sonnet");
        assert_eq!(version, Some(4.0));
    }

    #[test]
    fn test_parse_model_string_family_only() {
        let (family, version) = parse_model_string("opus");
        assert_eq!(family, "opus");
        assert_eq!(version, None);
    }

    #[test]
    fn test_parse_model_string_family_only_gpt() {
        let (family, version) = parse_model_string("gpt");
        assert_eq!(family, "gpt");
        assert_eq!(version, None);
    }

    #[test]
    fn test_parse_model_string_with_whitespace() {
        let (family, version) = parse_model_string("  sonnet4.5  ");
        assert_eq!(family, "sonnet");
        assert_eq!(version, Some(4.5));
    }

    #[test]
    fn test_parse_model_string_uppercase() {
        let (family, version) = parse_model_string("SONNET4.5");
        assert_eq!(family, "sonnet");
        assert_eq!(version, Some(4.5));
    }

    #[test]
    fn test_parse_model_string_mixed_case() {
        let (family, version) = parse_model_string("SoNnEt4.5");
        assert_eq!(family, "sonnet");
        assert_eq!(version, Some(4.5));
    }

    #[test]
    fn test_parse_model_string_haiku() {
        let (family, version) = parse_model_string("haiku4.5");
        assert_eq!(family, "haiku");
        assert_eq!(version, Some(4.5));
    }

    // ========================================================================
    // resolve_model Tests - Exact Match
    // ========================================================================

    #[test]
    fn test_resolve_model_exact_match_sonnet4() {
        let models = create_test_models();
        let result = resolve_model("sonnet4", &models).unwrap();
        assert_eq!(result, "sonnet4");
    }

    #[test]
    fn test_resolve_model_exact_match_sonnet4_5() {
        let models = create_test_models();
        let result = resolve_model("sonnet4.5", &models).unwrap();
        assert_eq!(result, "sonnet4.5");
    }

    #[test]
    fn test_resolve_model_exact_match_gpt5() {
        let models = create_test_models();
        let result = resolve_model("gpt5", &models).unwrap();
        assert_eq!(result, "gpt5");
    }

    #[test]
    fn test_resolve_model_exact_match_gpt5_1() {
        let models = create_test_models();
        let result = resolve_model("gpt5.1", &models).unwrap();
        assert_eq!(result, "gpt5.1");
    }

    // ========================================================================
    // resolve_model Tests - Family Only (picks highest version)
    // ========================================================================

    #[test]
    fn test_resolve_model_family_only_sonnet_picks_highest() {
        let models = create_test_models();
        let result = resolve_model("sonnet", &models).unwrap();
        // Should pick sonnet4.5 (highest version) over sonnet4
        assert_eq!(result, "sonnet4.5");
    }

    #[test]
    fn test_resolve_model_family_only_gpt_picks_highest() {
        let models = create_test_models();
        let result = resolve_model("gpt", &models).unwrap();
        // Should pick gpt5.1 (highest version) over gpt5
        assert_eq!(result, "gpt5.1");
    }

    #[test]
    fn test_resolve_model_family_only_haiku() {
        let models = create_test_models();
        let result = resolve_model("haiku", &models).unwrap();
        // Only one haiku model available
        assert_eq!(result, "haiku4.5");
    }

    #[test]
    fn test_resolve_model_family_only_opus() {
        let models = create_test_models();
        let result = resolve_model("opus", &models).unwrap();
        // Only one opus model available
        assert_eq!(result, "opus4.5");
    }

    // ========================================================================
    // resolve_model Tests - Error Cases
    // ========================================================================

    #[test]
    fn test_resolve_model_no_match_family() {
        let models = create_test_models();
        let result = resolve_model("llama", &models);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No model found"));
    }

    #[test]
    fn test_resolve_model_no_match_version() {
        let models = create_test_models();
        let result = resolve_model("sonnet3", &models);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No exact match"));
    }

    #[test]
    fn test_resolve_model_empty_input() {
        let models = create_test_models();
        let result = resolve_model("", &models);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty string"));
    }

    #[test]
    fn test_resolve_model_empty_models_list() {
        let models: Vec<AvailableModel> = vec![];
        let result = resolve_model("sonnet", &models);
        assert!(result.is_err());
    }

    // ========================================================================
    // resolve_model Tests - Case Insensitivity
    // ========================================================================

    #[test]
    fn test_resolve_model_case_insensitive() {
        let models = create_test_models();
        let result = resolve_model("SONNET4.5", &models).unwrap();
        assert_eq!(result, "sonnet4.5");
    }

    #[test]
    fn test_resolve_model_mixed_case() {
        let models = create_test_models();
        let result = resolve_model("SoNnEt", &models).unwrap();
        assert_eq!(result, "sonnet4.5");
    }

    // ========================================================================
    // Version Comparison Tests
    // ========================================================================

    #[test]
    fn test_version_ordering_5_1_greater_than_5() {
        // Verify that 5.1 > 5 when selecting highest version
        let models = create_test_models();

        // When asking for "gpt" family, should get gpt5.1 not gpt5
        let result = resolve_model("gpt", &models).unwrap();
        assert_eq!(result, "gpt5.1");
    }

    #[test]
    fn test_version_ordering_4_5_greater_than_4() {
        // Verify that 4.5 > 4 when selecting highest version
        let models = create_test_models();

        // When asking for "sonnet" family, should get sonnet4.5 not sonnet4
        let result = resolve_model("sonnet", &models).unwrap();
        assert_eq!(result, "sonnet4.5");
    }

    // ========================================================================
    // get_available_families Tests
    // ========================================================================

    #[test]
    fn test_get_available_families() {
        let models = create_test_models();
        let families = get_available_families(&models);

        assert!(families.contains(&"gpt".to_string()));
        assert!(families.contains(&"haiku".to_string()));
        assert!(families.contains(&"opus".to_string()));
        assert!(families.contains(&"sonnet".to_string()));
        assert_eq!(families.len(), 4); // 4 unique families
    }

    #[test]
    fn test_get_available_families_empty() {
        let models: Vec<AvailableModel> = vec![];
        let families = get_available_families(&models);
        assert!(families.is_empty());
    }
}

