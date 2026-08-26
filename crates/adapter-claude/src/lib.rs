mod native;
mod normalize;

pub use native::{ClaudeNativeEvent, ClaudeParseError, parse_stream_line};
pub use normalize::{ClaudeNormalizedEvent, ClaudeNormalizer, normalize_stream_line};
