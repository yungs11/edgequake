//! Shared LLM completion options for extraction extractors (SPEC-017 DRY-008).

use edgequake_llm::traits::CompletionOptions;

use super::temperature::effective_temperature_for_model;
use super::types::ExtractionResult;

/// Build extraction [`CompletionOptions`] for the entity/relation extraction phase.
///
/// reasoning 정책(관계 뽑을 때 추론 ON):
/// - `force_reasoning=Some(true)` → OpenRouter provider 에서 reasoning ON(전역 env off 무시).
///   qwen 관계 추출은 추론이 도움되고, max_tokens(16384) 여유로 빈-JSON 문제 미발생(실측).
/// - `reasoning_effort="none"` 은 **OpenAI o-series provider 보호용으로 유지**(그 모델들은
///   CoT 로 completion 예산을 소진해 빈 구조화 출력 → OpenAI provider 는 이 필드로 off).
///   OpenRouter 는 이 필드를 무시하므로 두 값이 provider 별로 각자 올바르게 동작한다.
pub fn extraction_completion_options(model: &str, max_tokens: usize) -> CompletionOptions {
    CompletionOptions {
        max_tokens: Some(max_tokens),
        temperature: effective_temperature_for_model(model, 0.0),
        reasoning_effort: Some("none".to_string()),
        force_reasoning: Some(true),
        ..Default::default()
    }
}

/// Adaptive chunk size recommendation based on document size (bytes).
pub fn recommended_chunk_size_for_bytes(chunk_size_bytes: usize) -> usize {
    if chunk_size_bytes > 100_000 {
        600
    } else if chunk_size_bytes > 50_000 {
        800
    } else {
        1200
    }
}

/// Copy token usage from an LLM response into an extraction result.
pub fn assign_token_usage(
    result: &mut ExtractionResult,
    input_tokens: usize,
    output_tokens: usize,
) {
    result.input_tokens = input_tokens;
    result.output_tokens = output_tokens;
}
