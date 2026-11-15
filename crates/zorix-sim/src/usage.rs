use zorix_core::{Message, ModelRequest, Usage};

/// Deterministic usage: tokens are estimated from the text and the price is one micro-dollar per token.
pub(crate) fn usage_for(request: &ModelRequest, output_chars: usize) -> Usage {
    let input_tokens: u64 = request.messages.iter().map(Message::estimated_tokens).sum();
    let output_tokens = (output_chars as u64).div_ceil(4);
    Usage {
        input_tokens,
        output_tokens,
        cost_micros: input_tokens + output_tokens,
        model_calls: 1,
        tool_calls: 0,
        ..Usage::default()
    }
}
