//! Token-by-token streaming decoder.
//!
//! Adapted from candle-examples `TokenOutputStream` — handles multi-byte
//! UTF-8 boundaries when decoding token IDs one at a time.

use tokenizers::Tokenizer;

#[allow(dead_code)]
pub struct TokenOutputStream {
    tokenizer: Tokenizer,
    tokens: Vec<u32>,
    prev_index: usize,
    current_index: usize,
}

#[allow(dead_code)]
impl TokenOutputStream {
    pub fn new(tokenizer: Tokenizer) -> Self {
        Self {
            tokenizer,
            tokens: Vec::new(),
            prev_index: 0,
            current_index: 0,
        }
    }

    fn decode(&self, tokens: &[u32]) -> anyhow::Result<String> {
        self.tokenizer
            .decode(tokens, true)
            .map_err(|e| anyhow::anyhow!("tokenizer decode: {e}"))
    }

    /// Feed one token. Returns `Some(text)` when new text is available,
    /// `None` when the bytes are still incomplete (mid-character).
    pub fn next_token(&mut self, token: u32) -> anyhow::Result<Option<String>> {
        let prev_text = if self.tokens.is_empty() {
            String::new()
        } else {
            let tokens = &self.tokens[self.prev_index..self.current_index];
            self.decode(tokens)?
        };
        self.tokens.push(token);
        let _current_index = self.current_index;
        self.current_index = self.tokens.len();
        let text = self.decode(&self.tokens[self.prev_index..])?;
        if text.len() > prev_text.len() && text.chars().last().map_or(false, |c| !c.is_whitespace() || c == ' ') {
            let new_text = text[prev_text.len()..].to_string();
            Ok(Some(new_text))
        } else if text.len() > prev_text.len() {
            // Whitespace character produced
            let new_text = text[prev_text.len()..].to_string();
            Ok(Some(new_text))
        } else {
            // No new text yet (incomplete multi-byte char)
            Ok(None)
        }
    }

    /// Flush any remaining bytes at end of generation.
    pub fn decode_rest(&self) -> anyhow::Result<Option<String>> {
        let prev_text = if self.tokens.is_empty() {
            return Ok(None);
        } else if self.prev_index >= self.tokens.len() {
            return Ok(None);
        } else {
            let prev_tokens = &self.tokens[self.prev_index..self.current_index.min(self.tokens.len())];
            if prev_tokens.is_empty() {
                String::new()
            } else {
                self.decode(prev_tokens)?
            }
        };
        let text = self.decode(&self.tokens[self.prev_index..])?;
        if text.len() > prev_text.len() {
            Ok(Some(text[prev_text.len()..].to_string()))
        } else {
            Ok(None)
        }
    }

    /// Get a token ID by its string representation.
    pub fn get_token(&self, token_s: &str) -> Option<u32> {
        self.tokenizer.token_to_id(token_s)
    }

    /// All tokens generated so far.
    pub fn tokens(&self) -> &[u32] {
        &self.tokens
    }

    pub fn clear(&mut self) {
        self.tokens.clear();
        self.prev_index = 0;
        self.current_index = 0;
    }

    pub fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }
}
