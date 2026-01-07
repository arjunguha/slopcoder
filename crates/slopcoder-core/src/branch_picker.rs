//! Feature branch name generation using DSRS (DSPy for Rust).

use dspy_rs::{configure, ChatAdapter, LM, Predict, Predictor, Signature, example};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BranchNameError {
    #[error("OPENAI_API_KEY is not set")]
    MissingApiKey,
    #[error("LLM call failed: {0}")]
    LlmFailed(String),
    #[error("LLM returned an empty branch name")]
    EmptyBranchName,
}

#[Signature]
struct BranchNameSignature {
    /// Task description for the new feature.
    #[input]
    task: String,
    /// A short git feature branch name, e.g. "fix-login-error". Do not use
    /// slashes or spaces. The feature name should not have more than 5 words.
    #[output]
    branch: String,
}

#[derive(Debug, Clone)]
struct BranchNameEnv {
    api_key: Option<String>,
    api_base: Option<String>,
}

impl BranchNameEnv {
    fn from_env() -> Self {
        let api_key = std::env::var("OPENAI_API_KEY")
            .ok()
            .and_then(|v| {
                let trimmed = v.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            });
        let api_base = std::env::var("OPENAI_API_BASE")
            .ok()
            .and_then(|v| {
                let trimmed = v.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            });

        Self { api_key, api_base }
    }
}

/// Generate a feature branch name based on a task description.
pub async fn pick_feature_branch(
    task_description: &str,
    model: &str,
) -> Result<String, BranchNameError> {
    let env = BranchNameEnv::from_env();
    if env.api_key.is_none() {
        return Err(BranchNameError::MissingApiKey);
    }

    let api_key = env
        .api_key
        .clone()
        .ok_or(BranchNameError::MissingApiKey)?;
    let lm = if let Some(base_url) = env.api_base.as_deref() {
        LM::builder()
            .model(model.to_string())
            .api_key(api_key)
            .base_url(base_url.to_string())
            .build()
            .await
            .map_err(|e| BranchNameError::LlmFailed(e.to_string()))?
    } else {
        LM::builder()
            .model(model.to_string())
            .api_key(api_key)
            .build()
            .await
            .map_err(|e| BranchNameError::LlmFailed(e.to_string()))?
    };
    configure(lm, ChatAdapter);

    let predictor = Predict::new(BranchNameSignature::new());
    let example = example! {
        "task": "input" => task_description,
    };
    let result = predictor
        .forward(example)
        .await
        .map_err(|e| BranchNameError::LlmFailed(e.to_string()))?;

    let raw = result.get("branch", None).as_str().unwrap_or("").to_string();
    normalize_branch_name(&raw).ok_or(BranchNameError::EmptyBranchName)
}

fn normalize_branch_name(raw: &str) -> Option<String> {
    let mut name = raw.trim();
    if name.is_empty() {
        return None;
    }
    if let Some(first_line) = name.lines().next() {
        name = first_line.trim();
    }
    name = name.trim_matches('`').trim_matches('"').trim_matches('\'');
    if name.is_empty() {
        return None;
    }

    let mut cleaned = String::new();
    for ch in name.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() || lower == '/' || lower == '-' || lower == '_' {
            cleaned.push(lower);
        } else if lower.is_whitespace() || lower == '.' {
            cleaned.push('-');
        }
    }

    let mut cleaned = cleaned
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if cleaned.is_empty() {
        return None;
    }

    if cleaned.len() > 40 {
        cleaned.truncate(40);
    }

    Some(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_branch_name_truncation() {
        let long_name = "this-is-a-very-long-branch-name-that-should-be-truncated-because-it-is-too-long";
        let normalized = normalize_branch_name(long_name).unwrap();
        assert!(normalized.len() <= 40);
        // It gets truncated to exactly 40 chars
        assert_eq!(normalized, "this-is-a-very-long-branch-name-that-sho");

        let short_name = "short-name";
        let normalized = normalize_branch_name(short_name).unwrap();
        assert_eq!(normalized, "short-name");
        
        let existing_feature = "feature/already-has-prefix";
        let normalized = normalize_branch_name(existing_feature).unwrap();
        assert_eq!(normalized, "feature/already-has-prefix");
    }

    #[tokio::test]
    async fn generates_feature_branch_name() {
        let model = "claude-haiku-4-5";
        let branch = pick_feature_branch(
            "Add a health check endpoint to the server and wire it into the router.",
            model,
        )
        .await
        .expect("branch generation failed");

        assert!(!branch.contains(' '));
        assert!(!branch.is_empty());
    }
}
