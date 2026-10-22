//! The Julia 1 input encoding.
//!
//! This follows the reference encoder in the `rust/` folder of
//! SupersonicLabs/Julia-1-ONNX (Apache 2.0). One row is one question with its
//! options and the shared state, laid out as:
//!
//! ```text
//! [BOS] head [SEP] ([MARKER] option)... [SEP] state [SEP]
//! ```
//!
//! The head is `"{type} question: {question}"`. Each marker position is
//! recorded, and the model returns one logit for each marker.

use tokenizers::Tokenizer;

/// First token of every sequence.
const BOS: u32 = 2;
/// Separator after the head, after the options and at the end.
const SEP: u32 = 1;
/// Token before each option; its position is the option's marker.
const MARKER: u32 = 4;
/// Text the model reserves for its markers.
const RESERVED: &str = "<mask>";
/// Most tokens of one option, without its marker.
const OPTION_TOKENS: usize = 48;
/// Below this head budget, the options are shortened.
const MIN_HEAD_BUDGET: i64 = 16;
/// The head keeps at least this many tokens.
const MIN_HEAD_TOKENS: i64 = 8;
/// Each shortened option keeps at least this many tokens, with its marker.
const MIN_OPTION_TOKENS: usize = 4;

/// Fewest and most options of one question.
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 20;

/// The question types and their `qtype` input values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Choice,
    Score,
    Noul,
}

impl Kind {
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "choice" => Some(Self::Choice),
            "score" => Some(Self::Score),
            "noul" => Some(Self::Noul),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Score => "score",
            Self::Noul => "noul",
        }
    }

    const fn qtype(self) -> u32 {
        match self {
            Self::Choice => 0,
            Self::Score => 1,
            Self::Noul => 2,
        }
    }
}

/// Token budgets and the truncation rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Most tokens of one row.
    pub max_length: usize,
    /// Tokens for the head and the options together.
    pub head_length: usize,
    /// Reject input that does not fit, instead of truncating it.
    pub strict: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_length: 8192,
            head_length: 512,
            strict: true,
        }
    }
}

/// One encoded row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    pub ids: Vec<u32>,
    pub markers: Vec<usize>,
    pub qtype: u32,
}

pub struct Encoder {
    // Boxed: a tokenizer is large, and encoders move between tasks.
    tokenizer: Box<Tokenizer>,
    limits: Limits,
}

impl std::fmt::Debug for Encoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encoder")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Encoder {
    /// Loads the tokenizer from the text of `tokenizer.json`.
    ///
    /// # Errors
    ///
    /// Fails when the tokenizer cannot be read, or the limits leave no room
    /// for the state.
    pub fn new(tokenizer_json: &str, limits: Limits) -> Result<Self, String> {
        if limits.head_length + 4 >= limits.max_length {
            return Err("head_length leaves no context".into());
        }
        let tokenizer = Tokenizer::from_bytes(tokenizer_json.as_bytes())
            .map_err(|error| format!("cannot load the tokenizer: {error}"))?;
        Ok(Self {
            tokenizer: Box::new(tokenizer),
            limits,
        })
    }

    fn tokens(&self, text: &str) -> Result<Vec<u32>, String> {
        let encoding = self
            .tokenizer
            .encode(text, false)
            .map_err(|error| format!("cannot tokenize: {error}"))?;
        Ok(encoding.get_ids().to_vec())
    }

    /// Removes the reserved marker text, or rejects it in strict mode.
    fn clean<'a>(&self, text: &'a str) -> Result<std::borrow::Cow<'a, str>, String> {
        if !text.contains(RESERVED) {
            return Ok(text.into());
        }
        if self.limits.strict {
            return Err("Reserved model marker in request".into());
        }
        Ok(text.replace(RESERVED, " ").into())
    }

    /// Encodes one question with its options and the shared state.
    ///
    /// # Errors
    ///
    /// Fails when the options are not valid, or the input does not fit the
    /// limits in strict mode.
    pub fn encode(
        &self,
        kind: Kind,
        question: &str,
        options: &[String],
        state: &str,
    ) -> Result<Encoded, String> {
        let count = options.len();
        if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&count)
            || options.iter().any(String::is_empty)
            || (kind == Kind::Noul && count != 2)
        {
            return Err("Options must contain 2-20 nonempty strings".into());
        }
        let head = self.tokens(&format!(
            "{} question: {}",
            kind.as_str(),
            self.clean(question)?
        ))?;
        let (blocks, budget) = self.option_blocks(&head, options)?;

        let mut ids = Vec::with_capacity(self.limits.max_length);
        ids.push(BOS);
        let head_tokens = usize::try_from(budget.max(MIN_HEAD_TOKENS)).unwrap_or(0);
        ids.extend(head.iter().take(head_tokens));
        ids.push(SEP);
        let mut markers = Vec::with_capacity(count);
        for block in &blocks {
            markers.push(ids.len());
            ids.extend(block);
        }
        ids.push(SEP);
        self.append_state(&mut ids, state)?;
        Ok(Encoded {
            ids,
            markers,
            qtype: kind.qtype(),
        })
    }

    /// Each option as its marker and tokens, fitted to the head budget, and
    /// the budget that is left for the head.
    fn option_blocks(
        &self,
        head: &[u32],
        options: &[String],
    ) -> Result<(Vec<Vec<u32>>, i64), String> {
        let Limits {
            head_length,
            strict,
            ..
        } = self.limits;
        let raw = self.option_tokens(options)?;
        if strict && raw.iter().any(|tokens| tokens.len() > OPTION_TOKENS) {
            return Err(format!(
                "Option exceeds {OPTION_TOKENS}-token model contract"
            ));
        }
        let mut blocks: Vec<Vec<u32>> = raw
            .iter()
            .map(|tokens| {
                std::iter::once(MARKER)
                    .chain(tokens.iter().copied().take(OPTION_TOKENS))
                    .collect()
            })
            .collect();
        let remaining = fit_to_head(&mut blocks, head_length);
        let truncated = blocks
            .iter()
            .zip(&raw)
            .any(|(block, tokens)| block.len() < tokens.len() + 1);
        if strict && (signed(head.len()) > remaining || truncated) {
            return Err("Question/options exceed lossless head budget".into());
        }
        Ok((blocks, remaining))
    }

    /// The tokens of each option, with a leading space.
    fn option_tokens(&self, options: &[String]) -> Result<Vec<Vec<u32>>, String> {
        options
            .iter()
            .map(|option| self.tokens(&format!(" {}", self.clean(option)?)))
            .collect()
    }

    /// Appends the state tokens that fit, and the closing separator.
    fn append_state(&self, ids: &mut Vec<u32>, state: &str) -> Result<(), String> {
        let state = self.tokens(&self.clean(state)?)?;
        let room = self.limits.max_length.saturating_sub(ids.len() + 1);
        if room < 1 {
            return Err("Question/options exceed sequence budget".into());
        }
        if self.limits.strict && state.len() > room {
            return Err("State exceeds lossless context budget".into());
        }
        ids.extend(state.iter().take(room));
        ids.push(SEP);
        Ok(())
    }
}

/// The head budget that `blocks` leave. Below [`MIN_HEAD_BUDGET`], each
/// block is first cut to an equal share of the head length.
fn fit_to_head(blocks: &mut [Vec<u32>], head_length: usize) -> i64 {
    let budget = |blocks: &[Vec<u32>]| {
        signed(head_length) - blocks.iter().map(|block| signed(block.len())).sum::<i64>()
    };
    let remaining = budget(blocks);
    if remaining >= MIN_HEAD_BUDGET || blocks.is_empty() {
        return remaining;
    }
    let cap = (head_length.saturating_sub(16) / blocks.len()).max(MIN_OPTION_TOKENS);
    for block in blocks.iter_mut() {
        block.truncate(cap);
    }
    budget(blocks)
}

fn signed(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A word-level tokenizer: each known word is one token, starting at 10.
    pub(crate) fn tokenizer_json() -> String {
        let words = [
            "choice", "noul", "score", "question", ":", "pick", "a", "b", "long", "state", "text",
        ];
        let vocab: serde_json::Map<String, serde_json::Value> = std::iter::once(("[UNK]", 3))
            .chain(words.iter().copied().zip(10..))
            .map(|(word, id)| (word.to_owned(), id.into()))
            .collect();
        serde_json::json!({
            "version": "1.0",
            "truncation": null,
            "padding": null,
            "added_tokens": [],
            "normalizer": null,
            "pre_tokenizer": {"type": "Whitespace"},
            "post_processor": null,
            "decoder": null,
            "model": {"type": "WordLevel", "vocab": vocab, "unk_token": "[UNK]"}
        })
        .to_string()
    }

    fn encoder(limits: Limits) -> Encoder {
        Encoder::new(&tokenizer_json(), limits).unwrap()
    }

    fn options(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn rows_follow_the_reference_layout() {
        let encoded = encoder(Limits::default())
            .encode(Kind::Choice, "pick", &options(&["a", "b"]), "state text")
            .unwrap();
        // BOS, "choice question: pick", SEP, MARKER a, MARKER b, SEP, "state text", SEP
        assert_eq!(
            encoded.ids,
            [2, 10, 13, 14, 15, 1, 4, 16, 4, 17, 1, 19, 20, 1]
        );
        assert_eq!(encoded.markers, [6, 8]);
        assert_eq!(encoded.qtype, 0);
        let noul = encoder(Limits::default())
            .encode(Kind::Noul, "pick", &options(&["a", "b"]), "state")
            .unwrap();
        assert_eq!((noul.ids[1], noul.qtype), (11, 2));
    }

    #[test]
    fn invalid_options_are_rejected() {
        let e = encoder(Limits::default());
        for bad in [options(&["a"]), options(&["a", ""]), options(&["a"; 21])] {
            assert!(
                e.encode(Kind::Choice, "pick", &bad, "s").is_err(),
                "{bad:?}"
            );
        }
        assert!(
            e.encode(Kind::Noul, "pick", &options(&["a", "b", "a"]), "s")
                .is_err()
        );
        assert!(
            Encoder::new(
                &tokenizer_json(),
                Limits {
                    max_length: 20,
                    head_length: 16,
                    strict: true
                }
            )
            .is_err()
        );
    }

    #[test]
    fn strict_mode_rejects_a_long_state_that_non_strict_mode_cuts() {
        let long_state = vec!["long"; 40].join(" ");
        let tight = Limits {
            max_length: 32,
            head_length: 20,
            strict: true,
        };
        let error = encoder(tight)
            .encode(Kind::Choice, "pick", &options(&["a", "b"]), &long_state)
            .unwrap_err();
        assert_eq!(error, "State exceeds lossless context budget");
        let cut = encoder(Limits {
            strict: false,
            ..tight
        })
        .encode(Kind::Choice, "pick", &options(&["a", "b"]), &long_state)
        .unwrap();
        assert_eq!(cut.ids.len(), 32);
        assert_eq!(cut.ids.last(), Some(&SEP));
    }

    #[test]
    fn strict_mode_rejects_the_reserved_marker_that_non_strict_mode_removes() {
        let marked = encoder(Limits::default()).encode(
            Kind::Choice,
            "pick <mask>",
            &options(&["a", "b"]),
            "s",
        );
        assert_eq!(marked.unwrap_err(), "Reserved model marker in request");
        let replaced = encoder(Limits {
            strict: false,
            ..Limits::default()
        })
        .encode(Kind::Choice, "pick <mask>", &options(&["a", "b"]), "s")
        .unwrap();
        assert_eq!(&replaced.ids[..6], &[2, 10, 13, 14, 15, 1]);
    }

    #[test]
    fn long_options_are_rejected_or_cut_to_the_contract() {
        let long = vec!["long"; 50].join(" ");
        let e = encoder(Limits::default());
        assert!(
            e.encode(Kind::Choice, "pick", &[long.clone(), "a".into()], "s")
                .is_err()
        );
        let cut = encoder(Limits {
            strict: false,
            ..Limits::default()
        })
        .encode(Kind::Choice, "pick", &[long, "a".into()], "s")
        .unwrap();
        assert_eq!(cut.markers[1] - cut.markers[0], OPTION_TOKENS + 1);
    }
}
