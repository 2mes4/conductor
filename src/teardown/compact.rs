//! Context compaction — prune LLM history to stay within token limits.
//!
//! Because OpenCode is amnesic, Rust controls the history. Using `tiktoken-rs`,
//! it counts tokens and, when the limit is exceeded, removes the densest
//! `tool_result` entries while preserving agent responses and user
//! instructions.

use serde_json::Value;
use tiktoken_rs::cl100k_base;

use crate::error::Result;

/// Count the total tokens in a JSON session history.
pub fn count_tokens(history: &Value) -> usize {
    let text = history.to_string();
    match cl100k_base() {
        Ok(bpe) => bpe.encode_with_special_tokens(&text).len(),
        Err(_) => text.len() / 4, // Fallback: ~4 chars per token.
    }
}

/// Compact the session history if it exceeds `max_tokens`.
///
/// Strategy:
/// 1. Walk the message array looking for `tool_result` entries.
/// 2. Sort them by size (largest first).
/// 3. Remove entries until we're under the budget.
/// 4. Replace removed entries with a placeholder so the conversation
///    structure stays valid.
///
/// Returns the compacted history and the number of tokens after compaction.
pub fn compact_history(history: &Value, max_tokens: usize) -> Result<(Value, usize)> {
    let current = count_tokens(history);
    if current <= max_tokens {
        return Ok((history.clone(), current));
    }

    tracing::info!(
        current_tokens = current,
        max_tokens,
        "compacting session history"
    );

    let mut compacted = history.clone();

    // Work on the "messages" array if present.
    if let Some(messages) = compacted.get_mut("messages").and_then(|m| m.as_array_mut()) {
        // Collect indices of tool_result messages with their estimated sizes.
        let mut tool_results: Vec<(usize, usize)> = messages
            .iter()
            .enumerate()
            .filter_map(|(i, msg)| {
                let role = msg.get("role").and_then(|r| r.as_str())?;
                if role == "tool" || role == "function" {
                    Some((i, count_tokens(msg)))
                } else {
                    None
                }
            })
            .collect();

        // Sort largest first — prune the biggest offenders first.
        tool_results.sort_by(|a, b| b.1.cmp(&a.1));

        for (idx, _size) in &tool_results {
            // Replace with a compact placeholder.
            if let Some(msg) = messages.get_mut(*idx) {
                if let Some(obj) = msg.as_object_mut() {
                    if let Some(content) = obj.get_mut("content") {
                        *content = Value::String("[compacted: tool output pruned to save context]".into());
                    }
                }
            }

            let new_count = count_tokens(&compacted);
            if new_count <= max_tokens {
                tracing::info!(new_tokens = new_count, "compaction complete");
                return Ok((compacted, new_count));
            }
        }
    }

    let final_count = count_tokens(&compacted);
    tracing::warn!(final_tokens = final_count, "could not reach token budget after compaction");
    Ok((compacted, final_count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_tokens_nonzero() {
        let history = json!({"messages": [{"role": "user", "content": "hello world"}]});
        assert!(count_tokens(&history) > 0);
    }

    #[test]
    fn no_compaction_under_limit() {
        let history = json!({"messages": [{"role": "user", "content": "hi"}]});
        let (result, tokens) = compact_history(&history, 100_000).unwrap();
        assert_eq!(result, history);
        assert!(tokens <= 100_000);
    }
}
