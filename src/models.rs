//! Model configuration and discovery for Villalobos
//!
//! This module provides types for managing AI model configuration using
//! tier-based model selection with fallback chains.
//!
//! # Model Tiers
//!
//! Instead of specific model versions (e.g., "sonnet4.5"), this module uses
//! model tiers (e.g., "sonnet") that each backend resolves to the best
//! available version.
//!
//! # Effort Levels
//!
//! Some backends (like Cursor) support effort/thinking levels that affect
//! model behavior. Use [`EffortLevel`] to request higher quality responses
//! at the cost of latency/tokens.
//!
//! # Fallback Chains
//!
//! Model configuration supports fallback chains like CSS font-family:
//! ```toml
//! orchestrator_model = ["gemini", "codex", "opus"]
//! ```
//! The system picks the first tier available in the current backend.

use anyhow::{anyhow, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::str::FromStr;

// ============================================================================
// Effort Level
// ============================================================================

/// Effort/thinking level for model inference.
///
/// Some backends support different effort levels that affect model quality
/// and latency. Higher effort typically means more "thinking" time and
/// better responses, but at the cost of increased latency and tokens.
///
/// # Backend Support
///
/// - **Cursor**: Maps to model suffixes like `-low`, `-high`, `-thinking`
/// - **Auggie**: Ignored (no effort level support)
///
/// # Example
///
/// ```toml
/// # .paperboat/agents/planner.toml
/// effort = "high"
/// model = "openai, opus, gemini, composer"
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EffortLevel {
    /// Low effort - fastest, minimal thinking
    Low,
    /// Medium effort - balanced (default)
    #[default]
    Medium,
    /// High effort - more thinking, better quality
    High,
    /// Extra high effort - maximum thinking/reasoning
    #[serde(rename = "xhigh")]
    XHigh,
}

impl EffortLevel {
    /// Returns the string identifier for this effort level.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }

    /// Returns all known effort levels.
    #[allow(dead_code)] // Public API for iteration over effort levels
    pub const fn all() -> &'static [Self] {
        &[Self::Low, Self::Medium, Self::High, Self::XHigh]
    }
}

impl FromStr for EffortLevel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "low" => Ok(Self::Low),
            "medium" | "med" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "xhigh" | "x-high" | "extra-high" | "max" => Ok(Self::XHigh),
            _ => Err(anyhow!("Unknown effort level: {s}")),
        }
    }
}

impl std::fmt::Display for EffortLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// Model Tiers
// ============================================================================

/// Model tiers representing capability classes across backends.
///
/// Each tier maps to the best available model version for that tier
/// in each backend. For example, `Sonnet` maps to:
/// - Auggie: `sonnet4.5` (or latest)
/// - Cursor: `sonnet-4.6` (or latest)
///
/// # Meta-Tiers
///
/// Some tiers are "meta-tiers" that expand to multiple concrete tiers:
/// - `OpenAI`: Expands to `[Gpt, Codex]` - all `OpenAI` models
///
/// # Effort Levels
///
/// When combined with [`EffortLevel`], backends may select different model
/// variants. For example, Cursor with `Gpt` + `High` effort resolves to
/// `gpt-5.4-high`.
///
/// Tiers are ordered roughly by capability (Opus > Sonnet > Haiku).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    /// Auto mode: system chooses based on task complexity
    Auto,
    /// Anthropic Opus - most capable, best for complex reasoning
    Opus,
    /// Anthropic Sonnet - balanced capability and speed
    #[default]
    Sonnet,
    /// Anthropic Haiku - fast and cheap (Auggie only)
    Haiku,
    /// `OpenAI` GPT - general purpose model (meta-tier for all GPT models)
    Gpt,
    /// `OpenAI` models - meta-tier that expands to [Gpt, Codex]
    #[serde(rename = "openai")]
    OpenAI,
    /// GPT Codex - coding-optimized model
    Codex,
    /// GPT Codex Mini - cheaper coding model
    CodexMini,
    /// Google Gemini Pro
    Gemini,
    /// Google Gemini Flash - faster/cheaper
    GeminiFlash,
    /// Grok by xAI
    Grok,
    /// Cursor Composer - Cursor's custom model
    Composer,
}

impl ModelTier {
    /// Returns the string identifier for this tier (used in config files).
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Opus => "opus",
            Self::Sonnet => "sonnet",
            Self::Haiku => "haiku",
            Self::Gpt => "gpt",
            Self::OpenAI => "openai",
            Self::Codex => "codex",
            Self::CodexMini => "codex-mini",
            Self::Gemini => "gemini",
            Self::GeminiFlash => "gemini-flash",
            Self::Grok => "grok",
            Self::Composer => "composer",
        }
    }

    /// Returns all known model tiers.
    #[allow(dead_code)] // Public API for iteration over model tiers
    pub const fn all() -> &'static [Self] {
        &[
            Self::Auto,
            Self::Opus,
            Self::Sonnet,
            Self::Haiku,
            Self::Gpt,
            Self::OpenAI,
            Self::Codex,
            Self::CodexMini,
            Self::Gemini,
            Self::GeminiFlash,
            Self::Grok,
            Self::Composer,
        ]
    }

    /// Returns `true` if this is the `Auto` variant.
    #[allow(dead_code)] // Public API for tier introspection
    pub const fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    /// Returns `true` if this is a meta-tier that expands to multiple tiers.
    #[allow(dead_code)] // Public API for meta-tier detection
    pub const fn is_meta_tier(&self) -> bool {
        matches!(self, Self::OpenAI)
    }

    /// Expands a meta-tier to its constituent tiers.
    ///
    /// For example, `OpenAI` expands to `[Gpt, Codex]`.
    /// Non-meta tiers return a single-element slice containing themselves.
    #[allow(dead_code)] // Public API for meta-tier expansion
    pub fn expand(&self) -> Vec<Self> {
        match self {
            Self::OpenAI => vec![Self::Gpt, Self::Codex],
            other => vec![*other],
        }
    }

    /// Resolves the "auto" tier to a concrete tier based on complexity.
    ///
    /// - Simple → Haiku (fast, cheap)
    /// - Medium → Sonnet (balanced)
    /// - Complex → Opus (most capable)
    /// - None → Sonnet (safe default)
    #[allow(clippy::missing_const_for_fn)] // const fn not possible with `use` statement
    pub fn resolve_auto(&self, complexity: Option<crate::mcp_server::ModelComplexity>) -> Self {
        use crate::mcp_server::ModelComplexity;

        if !self.is_auto() {
            return *self;
        }

        match complexity {
            Some(ModelComplexity::Simple) => Self::Haiku,
            Some(ModelComplexity::Medium) | None => Self::Sonnet,
            Some(ModelComplexity::Complex) => Self::Opus,
        }
    }
}

impl FromStr for ModelTier {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "opus" => Ok(Self::Opus),
            "sonnet" => Ok(Self::Sonnet),
            "haiku" => Ok(Self::Haiku),
            "gpt" => Ok(Self::Gpt),
            "openai" => Ok(Self::OpenAI),
            "codex" => Ok(Self::Codex),
            "codex-mini" | "codexmini" => Ok(Self::CodexMini),
            "gemini" => Ok(Self::Gemini),
            "gemini-flash" | "geminiflash" => Ok(Self::GeminiFlash),
            "grok" => Ok(Self::Grok),
            "composer" => Ok(Self::Composer),
            _ => Err(anyhow!("Unknown model tier: {s}")),
        }
    }
}

impl std::fmt::Display for ModelTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A fallback chain of model tiers.
///
/// When selecting a model, the system tries each tier in order and
/// uses the first one available in the current backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelFallbackChain(pub Vec<ModelTier>);

impl ModelFallbackChain {
    /// Create a new fallback chain from a list of tiers.
    #[allow(clippy::missing_const_for_fn)]
    pub fn new(tiers: Vec<ModelTier>) -> Self {
        Self(tiers)
    }

    /// Create a single-tier chain (no fallbacks).
    pub fn single(tier: ModelTier) -> Self {
        Self(vec![tier])
    }

    /// Resolve this fallback chain to a concrete tier using available tiers.
    ///
    /// Returns the first tier in the chain that is available in the given set.
    /// Returns an error if no tier in the chain is available.
    pub fn resolve(&self, available: &HashSet<ModelTier>) -> Result<ModelTier> {
        for tier in &self.0 {
            if tier.is_auto() || available.contains(tier) {
                return Ok(*tier);
            }
        }
        Err(anyhow!(
            "No model tier in fallback chain {:?} is available. Available tiers: {:?}",
            self.0.iter().map(ModelTier::as_str).collect::<Vec<_>>(),
            available.iter().map(ModelTier::as_str).collect::<Vec<_>>()
        ))
    }

    /// Returns `true` if the chain contains only `Auto`.
    #[allow(dead_code)] // Public API for chain introspection
    pub fn is_auto(&self) -> bool {
        self.0.len() == 1 && self.0[0].is_auto()
    }

    /// Get the first tier in the chain (for display purposes).
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub fn primary(&self) -> Option<ModelTier> {
        self.0.first().copied()
    }
}

impl Default for ModelFallbackChain {
    fn default() -> Self {
        Self::single(ModelTier::default())
    }
}

impl std::fmt::Display for ModelFallbackChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tiers: Vec<&str> = self.0.iter().map(ModelTier::as_str).collect();
        write!(f, "[{}]", tiers.join(", "))
    }
}

impl FromStr for ModelFallbackChain {
    type Err = anyhow::Error;

    /// Parse a fallback chain from a string.
    ///
    /// Accepts either:
    /// - Single tier: "sonnet"
    /// - Comma-separated list: "gemini, codex, opus"
    fn from_str(s: &str) -> Result<Self> {
        let tiers: Result<Vec<ModelTier>> = s
            .split(',')
            .map(|t| ModelTier::from_str(t.trim()))
            .collect();
        Ok(Self(tiers?))
    }
}

/// Configuration for which models to use for different roles.
///
/// Each role has a fallback chain of model tiers. At runtime, the system
/// resolves each chain to a concrete model using the backend's available tiers.
///
/// # Effort Levels
///
/// Each agent role can have its own effort level. When the backend supports
/// effort levels (e.g., Cursor), this affects the model variant selected.
/// For example, `Opus` with `High` effort might resolve to `opus-4.6-thinking`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Available model tiers from the backend
    pub available_tiers: HashSet<ModelTier>,
    /// Model fallback chain for orchestration (default: opus, sonnet)
    pub orchestrator_model: ModelFallbackChain,
    /// Effort level for orchestrator (default: medium)
    pub orchestrator_effort: EffortLevel,
    /// Model fallback chain for planning (default: sonnet, opus)
    pub planner_model: ModelFallbackChain,
    /// Effort level for planner (default: medium)
    pub planner_effort: EffortLevel,
    /// Model fallback chain for implementation (default: sonnet, codex)
    pub implementer_model: ModelFallbackChain,
    /// Effort level for implementer (default: medium)
    pub implementer_effort: EffortLevel,
}

/// Default fallback chains for different environments.
pub mod defaults {
    use super::{ModelFallbackChain, ModelTier};

    /// Default orchestrator chain: prefer most capable models
    pub fn orchestrator() -> ModelFallbackChain {
        ModelFallbackChain::new(vec![ModelTier::Opus, ModelTier::Sonnet, ModelTier::Codex])
    }

    /// Default planner chain: balanced capability
    pub fn planner() -> ModelFallbackChain {
        ModelFallbackChain::new(vec![ModelTier::Sonnet, ModelTier::Opus, ModelTier::Codex])
    }

    /// Default implementer chain: coding-optimized
    pub fn implementer() -> ModelFallbackChain {
        ModelFallbackChain::new(vec![ModelTier::Sonnet, ModelTier::Codex, ModelTier::Opus])
    }
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            available_tiers: HashSet::new(),
            orchestrator_model: defaults::orchestrator(),
            orchestrator_effort: EffortLevel::default(),
            planner_model: defaults::planner(),
            planner_effort: EffortLevel::default(),
            implementer_model: defaults::implementer(),
            implementer_effort: EffortLevel::default(),
        }
    }
}

impl ModelConfig {
    /// Creates a new `ModelConfig` with given available tiers and default chains.
    pub fn new(available_tiers: HashSet<ModelTier>) -> Self {
        Self {
            available_tiers,
            orchestrator_model: defaults::orchestrator(),
            orchestrator_effort: EffortLevel::default(),
            planner_model: defaults::planner(),
            planner_effort: EffortLevel::default(),
            implementer_model: defaults::implementer(),
            implementer_effort: EffortLevel::default(),
        }
    }

    /// Applies debug build model override.
    ///
    /// In debug builds, all models use the cheap fallback chain for fast testing.
    /// This can be overridden by setting the `PAPERBOAT_MODEL` environment variable.
    ///
    /// In release builds, this is a no-op (respects user configuration).
    #[cfg(debug_assertions)]
    pub fn apply_debug_override(&mut self) {
        // Check for environment variable override first
        if let Ok(model_str) = std::env::var("PAPERBOAT_MODEL") {
            if let Ok(tier) = ModelTier::from_str(&model_str) {
                let chain = ModelFallbackChain::single(tier);
                tracing::info!("🧪 PAPERBOAT_MODEL override: using {} for all agents", tier);
                self.orchestrator_model = chain.clone();
                self.planner_model = chain.clone();
                self.implementer_model = chain;
                return;
            }
            tracing::warn!(
                "⚠️  Invalid PAPERBOAT_MODEL '{}', falling back to debug default",
                model_str
            );
        }

        // Debug build default: use cheap fallback chain
        let cheap = ModelFallbackChain::new(vec![
            ModelTier::CodexMini,
            ModelTier::Grok,
            ModelTier::GeminiFlash,
            ModelTier::Haiku,
        ]);
        tracing::info!(
            "🧪 Debug build: using cheap models {} (override with PAPERBOAT_MODEL)",
            cheap
        );
        self.orchestrator_model = cheap.clone();
        self.planner_model = cheap.clone();
        self.implementer_model = cheap;
    }

    /// Applies debug build model override (no-op in release builds).
    #[cfg(not(debug_assertions))]
    #[allow(clippy::missing_const_for_fn)]
    pub fn apply_debug_override(&mut self) {
        // Release build: respect user configuration
    }

    /// Validates that at least one tier in each fallback chain is available.
    pub fn validate(&self) -> Result<()> {
        // Try to resolve each chain - this will error if no tier is available
        self.orchestrator_model
            .resolve(&self.available_tiers)
            .map_err(|_| anyhow!("No orchestrator model tier is available"))?;

        self.planner_model
            .resolve(&self.available_tiers)
            .map_err(|_| anyhow!("No planner model tier is available"))?;

        self.implementer_model
            .resolve(&self.available_tiers)
            .map_err(|_| anyhow!("No implementer model tier is available"))?;

        Ok(())
    }

    /// Resolve the orchestrator model chain to a concrete tier.
    #[allow(dead_code)] // Public API for model resolution
    pub fn resolve_orchestrator(&self) -> Result<ModelTier> {
        self.orchestrator_model.resolve(&self.available_tiers)
    }

    /// Resolve the planner model chain to a concrete tier.
    #[allow(dead_code)] // Public API for model resolution
    pub fn resolve_planner(&self) -> Result<ModelTier> {
        self.planner_model.resolve(&self.available_tiers)
    }

    /// Resolve the implementer model chain to a concrete tier.
    #[allow(dead_code)] // Public API for model resolution
    pub fn resolve_implementer(&self) -> Result<ModelTier> {
        self.implementer_model.resolve(&self.available_tiers)
    }
}

/// Parses the output of `auggie model list` and returns available tiers.
///
/// This function parses the model list output format and extracts
/// the tier from model IDs like "haiku4.5", "sonnet4.5", "opus4.5".
///
/// # Arguments
///
/// * `output` - The raw output from `auggie model list`
///
/// # Returns
///
/// A set of available `ModelTier`s parsed from the output.
pub fn parse_auggie_model_list(output: &str) -> Result<HashSet<ModelTier>> {
    let mut tiers = HashSet::new();

    // Pattern to match lines like " - Haiku 4.5 [haiku4.5]"
    let model_re = Regex::new(r"^\s*-\s*.+?\s*\[([^\]]+)\]\s*$")?;

    for line in output.lines() {
        if let Some(caps) = model_re.captures(line) {
            let id_str = caps.get(1).map_or("", |m| m.as_str()).trim();
            // Extract tier from model ID (e.g., "haiku4.5" -> "haiku")
            if let Some(tier) = extract_tier_from_auggie_id(id_str) {
                tiers.insert(tier);
            }
        }
    }

    Ok(tiers)
}

/// Extract a `ModelTier` from an Auggie model ID string.
///
/// Examples:
/// - "haiku4.5" -> Haiku
/// - "sonnet4.5" -> Sonnet
/// - "opus4.5" -> Opus
fn extract_tier_from_auggie_id(id: &str) -> Option<ModelTier> {
    let lower = id.to_lowercase();
    if lower.starts_with("haiku") {
        Some(ModelTier::Haiku)
    } else if lower.starts_with("sonnet") {
        Some(ModelTier::Sonnet)
    } else if lower.starts_with("opus") {
        Some(ModelTier::Opus)
    } else if lower.starts_with("gpt") {
        Some(ModelTier::Codex) // Map GPT models to Codex tier
    } else if lower == "auto" {
        Some(ModelTier::Auto)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // EffortLevel Tests
    // ========================================================================

    #[test]
    fn test_effort_level_as_str() {
        assert_eq!(EffortLevel::Low.as_str(), "low");
        assert_eq!(EffortLevel::Medium.as_str(), "medium");
        assert_eq!(EffortLevel::High.as_str(), "high");
        assert_eq!(EffortLevel::XHigh.as_str(), "xhigh");
    }

    #[test]
    fn test_effort_level_from_str() {
        assert_eq!(EffortLevel::from_str("low").unwrap(), EffortLevel::Low);
        assert_eq!(
            EffortLevel::from_str("medium").unwrap(),
            EffortLevel::Medium
        );
        assert_eq!(EffortLevel::from_str("med").unwrap(), EffortLevel::Medium);
        assert_eq!(EffortLevel::from_str("high").unwrap(), EffortLevel::High);
        assert_eq!(EffortLevel::from_str("xhigh").unwrap(), EffortLevel::XHigh);
        assert_eq!(EffortLevel::from_str("x-high").unwrap(), EffortLevel::XHigh);
        assert_eq!(
            EffortLevel::from_str("extra-high").unwrap(),
            EffortLevel::XHigh
        );
        assert_eq!(EffortLevel::from_str("max").unwrap(), EffortLevel::XHigh);
    }

    #[test]
    fn test_effort_level_from_str_case_insensitive() {
        assert_eq!(EffortLevel::from_str("LOW").unwrap(), EffortLevel::Low);
        assert_eq!(EffortLevel::from_str("HIGH").unwrap(), EffortLevel::High);
        assert_eq!(EffortLevel::from_str("XHIGH").unwrap(), EffortLevel::XHigh);
    }

    #[test]
    fn test_effort_level_from_str_invalid() {
        assert!(EffortLevel::from_str("invalid").is_err());
        assert!(EffortLevel::from_str("").is_err());
        assert!(EffortLevel::from_str("highest").is_err());
    }

    #[test]
    fn test_effort_level_default() {
        assert_eq!(EffortLevel::default(), EffortLevel::Medium);
    }

    #[test]
    fn test_effort_level_display() {
        assert_eq!(format!("{}", EffortLevel::Low), "low");
        assert_eq!(format!("{}", EffortLevel::Medium), "medium");
        assert_eq!(format!("{}", EffortLevel::High), "high");
        assert_eq!(format!("{}", EffortLevel::XHigh), "xhigh");
    }

    #[test]
    fn test_effort_level_serde_roundtrip() {
        let levels = [
            EffortLevel::Low,
            EffortLevel::Medium,
            EffortLevel::High,
            EffortLevel::XHigh,
        ];

        for level in levels {
            let json = serde_json::to_string(&level).unwrap();
            let parsed: EffortLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(level, parsed);
        }
    }

    // ========================================================================
    // ModelTier Tests
    // ========================================================================

    #[test]
    fn test_model_tier_as_str() {
        assert_eq!(ModelTier::Auto.as_str(), "auto");
        assert_eq!(ModelTier::Opus.as_str(), "opus");
        assert_eq!(ModelTier::Sonnet.as_str(), "sonnet");
        assert_eq!(ModelTier::Haiku.as_str(), "haiku");
        assert_eq!(ModelTier::Gpt.as_str(), "gpt");
        assert_eq!(ModelTier::OpenAI.as_str(), "openai");
        assert_eq!(ModelTier::Codex.as_str(), "codex");
        assert_eq!(ModelTier::CodexMini.as_str(), "codex-mini");
    }

    #[test]
    fn test_model_tier_from_str() {
        assert_eq!(ModelTier::from_str("auto").unwrap(), ModelTier::Auto);
        assert_eq!(ModelTier::from_str("opus").unwrap(), ModelTier::Opus);
        assert_eq!(ModelTier::from_str("sonnet").unwrap(), ModelTier::Sonnet);
        assert_eq!(ModelTier::from_str("SONNET").unwrap(), ModelTier::Sonnet); // case insensitive
        assert_eq!(ModelTier::from_str("gpt").unwrap(), ModelTier::Gpt);
        assert_eq!(ModelTier::from_str("openai").unwrap(), ModelTier::OpenAI);
        assert_eq!(
            ModelTier::from_str("codex-mini").unwrap(),
            ModelTier::CodexMini
        );
        assert_eq!(
            ModelTier::from_str("codexmini").unwrap(),
            ModelTier::CodexMini
        );
    }

    #[test]
    fn test_model_tier_meta_tier_expand() {
        // OpenAI is a meta-tier that expands to Gpt and Codex
        let expanded = ModelTier::OpenAI.expand();
        assert_eq!(expanded, vec![ModelTier::Gpt, ModelTier::Codex]);

        // Non-meta tiers expand to just themselves
        let expanded = ModelTier::Opus.expand();
        assert_eq!(expanded, vec![ModelTier::Opus]);

        let expanded = ModelTier::Sonnet.expand();
        assert_eq!(expanded, vec![ModelTier::Sonnet]);
    }

    #[test]
    fn test_model_tier_is_meta_tier() {
        assert!(ModelTier::OpenAI.is_meta_tier());
        assert!(!ModelTier::Gpt.is_meta_tier());
        assert!(!ModelTier::Opus.is_meta_tier());
        assert!(!ModelTier::Sonnet.is_meta_tier());
    }

    #[test]
    fn test_model_tier_from_str_invalid() {
        assert!(ModelTier::from_str("invalid").is_err());
        assert!(ModelTier::from_str("").is_err());
    }

    #[test]
    fn test_model_tier_default() {
        assert_eq!(ModelTier::default(), ModelTier::Sonnet);
    }

    #[test]
    fn test_model_tier_is_auto() {
        assert!(ModelTier::Auto.is_auto());
        assert!(!ModelTier::Sonnet.is_auto());
        assert!(!ModelTier::Opus.is_auto());
    }

    #[test]
    fn test_model_tier_resolve_auto() {
        use crate::mcp_server::ModelComplexity;

        assert_eq!(
            ModelTier::Auto.resolve_auto(Some(ModelComplexity::Simple)),
            ModelTier::Haiku
        );
        assert_eq!(
            ModelTier::Auto.resolve_auto(Some(ModelComplexity::Medium)),
            ModelTier::Sonnet
        );
        assert_eq!(
            ModelTier::Auto.resolve_auto(Some(ModelComplexity::Complex)),
            ModelTier::Opus
        );
        assert_eq!(ModelTier::Auto.resolve_auto(None), ModelTier::Sonnet);
    }

    #[test]
    fn test_model_tier_resolve_auto_non_auto_unchanged() {
        use crate::mcp_server::ModelComplexity;

        assert_eq!(
            ModelTier::Sonnet.resolve_auto(Some(ModelComplexity::Complex)),
            ModelTier::Sonnet
        );
        assert_eq!(
            ModelTier::Opus.resolve_auto(Some(ModelComplexity::Simple)),
            ModelTier::Opus
        );
    }

    #[test]
    fn test_model_tier_serde_roundtrip() {
        for tier in ModelTier::all() {
            let json = serde_json::to_string(tier).unwrap();
            let parsed: ModelTier = serde_json::from_str(&json).unwrap();
            assert_eq!(*tier, parsed);
        }
    }

    // ========================================================================
    // ModelFallbackChain Tests
    // ========================================================================

    #[test]
    fn test_fallback_chain_single() {
        let chain = ModelFallbackChain::single(ModelTier::Sonnet);
        assert_eq!(chain.0.len(), 1);
        assert_eq!(chain.primary(), Some(ModelTier::Sonnet));
    }

    #[test]
    fn test_fallback_chain_resolve_first_available() {
        let chain =
            ModelFallbackChain::new(vec![ModelTier::Gemini, ModelTier::Codex, ModelTier::Opus]);

        // Only Codex and Opus available
        let available: HashSet<ModelTier> =
            [ModelTier::Codex, ModelTier::Opus].into_iter().collect();

        let resolved = chain.resolve(&available).unwrap();
        assert_eq!(resolved, ModelTier::Codex); // First available
    }

    #[test]
    fn test_fallback_chain_resolve_none_available() {
        let chain = ModelFallbackChain::new(vec![ModelTier::Gemini, ModelTier::Grok]);

        let available: HashSet<ModelTier> = std::iter::once(ModelTier::Sonnet).collect();

        assert!(chain.resolve(&available).is_err());
    }

    #[test]
    fn test_fallback_chain_auto_always_resolves() {
        let chain = ModelFallbackChain::single(ModelTier::Auto);
        let available: HashSet<ModelTier> = HashSet::new(); // Empty!

        // Auto should always resolve, even with no available tiers
        let resolved = chain.resolve(&available).unwrap();
        assert_eq!(resolved, ModelTier::Auto);
    }

    #[test]
    fn test_fallback_chain_from_str_single() {
        let chain: ModelFallbackChain = "sonnet".parse().unwrap();
        assert_eq!(chain.0, vec![ModelTier::Sonnet]);
    }

    #[test]
    fn test_fallback_chain_from_str_multiple() {
        let chain: ModelFallbackChain = "gemini, codex, opus".parse().unwrap();
        assert_eq!(
            chain.0,
            vec![ModelTier::Gemini, ModelTier::Codex, ModelTier::Opus]
        );
    }

    #[test]
    fn test_fallback_chain_display() {
        let chain = ModelFallbackChain::new(vec![ModelTier::Sonnet, ModelTier::Opus]);
        assert_eq!(format!("{chain}"), "[sonnet, opus]");
    }

    // ========================================================================
    // ModelConfig Tests
    // ========================================================================

    #[test]
    fn test_model_config_default() {
        let config = ModelConfig::default();
        assert!(config.available_tiers.is_empty());
    }

    #[test]
    fn test_model_config_validate_success() {
        let available: HashSet<ModelTier> = [ModelTier::Opus, ModelTier::Sonnet, ModelTier::Codex]
            .into_iter()
            .collect();

        let config = ModelConfig::new(available);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_model_config_validate_fails_when_no_tier_available() {
        // Config with only Gemini available, but default chains don't include it
        let available: HashSet<ModelTier> = std::iter::once(ModelTier::Gemini).collect();

        let config = ModelConfig::new(available);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_model_config_resolve_tiers() {
        let available: HashSet<ModelTier> =
            [ModelTier::Sonnet, ModelTier::Codex].into_iter().collect();

        let mut config = ModelConfig::new(available);
        config.orchestrator_model =
            ModelFallbackChain::new(vec![ModelTier::Opus, ModelTier::Sonnet]);

        // Should resolve to Sonnet (first available in chain)
        assert_eq!(config.resolve_orchestrator().unwrap(), ModelTier::Sonnet);
    }

    // ========================================================================
    // parse_auggie_model_list Tests
    // ========================================================================

    #[test]
    fn test_parse_auggie_model_list() {
        let output = r" - Haiku 4.5 [haiku4.5]
      Fast and efficient responses
 - Opus 4.5 [opus4.5]
      Best for complex tasks
 - Sonnet 4.5 [sonnet4.5]
      Great for everyday tasks";

        let tiers = parse_auggie_model_list(output).unwrap();

        assert!(tiers.contains(&ModelTier::Haiku));
        assert!(tiers.contains(&ModelTier::Opus));
        assert!(tiers.contains(&ModelTier::Sonnet));
    }

    #[test]
    fn test_parse_auggie_model_list_empty() {
        let tiers = parse_auggie_model_list("").unwrap();
        assert!(tiers.is_empty());
    }

    // ========================================================================
    // Debug Override Tests
    // ========================================================================

    #[cfg(debug_assertions)]
    use std::sync::Mutex;
    #[cfg(debug_assertions)]
    static ENV_VAR_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    #[cfg(debug_assertions)]
    fn test_apply_debug_override_sets_cheap_chain() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        std::env::remove_var("PAPERBOAT_MODEL");

        let mut config = ModelConfig::default();
        config.apply_debug_override();

        // Should use cheap chain (codex-mini, grok, gemini-flash, haiku)
        assert_eq!(
            config.orchestrator_model.primary(),
            Some(ModelTier::CodexMini)
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    fn test_apply_debug_override_respects_env_var() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();
        std::env::set_var("PAPERBOAT_MODEL", "sonnet");

        let mut config = ModelConfig::default();
        config.apply_debug_override();

        assert_eq!(config.orchestrator_model.primary(), Some(ModelTier::Sonnet));

        std::env::remove_var("PAPERBOAT_MODEL");
    }
}
