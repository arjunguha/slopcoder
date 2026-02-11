//! Task topic-name generation using DSRS (DSPy for Rust).

use dspy_rs::{configure, example, ChatAdapter, Predict, Predictor, Signature, LM};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TopicNameError {
    #[error("OPENAI_API_KEY is not set")]
    MissingApiKey,
    #[error("LLM call failed: {0}")]
    LlmFailed(String),
    #[error("LLM returned an empty topic")]
    EmptyTopic,
}

#[Signature]
struct TopicNameSignature {
    /// Task prompt to summarize.
    #[input]
    prompt: String,
    /// A short topic name for this task, 20 characters max.
    #[output]
    topic: String,
}

#[derive(Debug, Clone)]
struct TopicNameEnv {
    api_key: Option<String>,
    api_base: Option<String>,
}

impl TopicNameEnv {
    fn from_env() -> Self {
        let api_key = std::env::var("OPENAI_API_KEY").ok().and_then(|v| {
            let trimmed = v.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        let api_base = std::env::var("OPENAI_API_BASE").ok().and_then(|v| {
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

/// Generate a short task topic name from a prompt.
pub async fn pick_task_topic(prompt: &str, model: &str) -> Result<String, TopicNameError> {
    pick_task_topic_with_env(prompt, model, TopicNameEnv::from_env()).await
}

async fn pick_task_topic_with_env(
    prompt: &str,
    model: &str,
    env: TopicNameEnv,
) -> Result<String, TopicNameError> {
    if env.api_key.is_none() {
        return Err(TopicNameError::MissingApiKey);
    }

    let api_key = env.api_key.clone().ok_or(TopicNameError::MissingApiKey)?;
    let lm = if let Some(base_url) = env.api_base.as_deref() {
        LM::builder()
            .model(model.to_string())
            .api_key(api_key)
            .base_url(base_url.to_string())
            .build()
            .await
            .map_err(|e| TopicNameError::LlmFailed(e.to_string()))?
    } else {
        LM::builder()
            .model(model.to_string())
            .api_key(api_key)
            .build()
            .await
            .map_err(|e| TopicNameError::LlmFailed(e.to_string()))?
    };
    configure(lm, ChatAdapter);

    let predictor = Predict::new(TopicNameSignature::new());
    let example = example! {
        "prompt": "input" => prompt,
    };
    let result = predictor
        .forward(example)
        .await
        .map_err(|e| TopicNameError::LlmFailed(e.to_string()))?;

    let raw = result.get("topic", None).as_str().unwrap_or("").to_string();
    normalize_topic_name(&raw).ok_or(TopicNameError::EmptyTopic)
}

pub fn fallback_topic_name(prompt: &str) -> String {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return "task".to_string();
    }

    let first_line = trimmed.lines().next().unwrap_or("").trim();
    let mut topic = first_line.chars().take(20).collect::<String>();
    topic = topic.trim().to_string();
    if topic.is_empty() {
        "task".to_string()
    } else {
        topic
    }
}

pub fn topic_to_branch_slug(topic: &str) -> String {
    let mut cleaned = String::new();

    for ch in topic.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() || lower == '-' || lower == '_' {
            cleaned.push(lower);
        } else if lower.is_whitespace() || lower == '.' {
            cleaned.push('-');
        }
    }

    let compact = cleaned
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if compact.is_empty() {
        "task".to_string()
    } else {
        compact
    }
}

fn normalize_topic_name(raw: &str) -> Option<String> {
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

    // Keep readable topic names; collapse repeated whitespace.
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }

    let mut normalized = words.join(" ");
    if normalized.len() > 20 {
        normalized.truncate(20);
        normalized = normalized.trim().to_string();
    }

    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_topic_name_truncation() {
        let long_name = "Implement websocket auth and coordinator routing";
        let normalized = normalize_topic_name(long_name).unwrap();
        assert_eq!(normalized, "Implement websocket");
        assert!(normalized.len() <= 20);

        let short_name = "Fix login";
        let normalized = normalize_topic_name(short_name).unwrap();
        assert_eq!(normalized, "Fix login");
    }

    #[test]
    fn test_fallback_topic_name() {
        assert_eq!(fallback_topic_name(""), "task");
        assert_eq!(
            fallback_topic_name("Build support for isolated worktrees now"),
            "Build support for is"
        );
    }

    #[test]
    fn test_topic_to_branch_slug() {
        assert_eq!(topic_to_branch_slug("Fix login flow"), "fix-login-flow");
        assert_eq!(topic_to_branch_slug("   ???   "), "task");
    }

    #[tokio::test]
    async fn returns_missing_api_key_without_key() {
        let env = TopicNameEnv {
            api_key: None,
            api_base: None,
        };
        let model = "claude-haiku-4-5";
        let err = pick_task_topic_with_env(
            "Add a health check endpoint to the server and wire it into the router.",
            model,
            env,
        )
        .await
        .expect_err("expected MissingApiKey");
        assert!(matches!(err, TopicNameError::MissingApiKey));
    }
}
