#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The encoder against the token IDs of the reference encoder in
//! Julia-1-ONNX. It needs the real tokenizer, so it is ignored by default:
//!
//! ```sh
//! saci-julia download
//! SACI_JULIA_MODEL_DIR=~/.cache/saci-julia/<revision> \
//!     cargo test -p saci-julia --test reference -- --ignored
//! ```

use saci_julia::encode::{Encoder, Kind, Limits};

#[test]
#[ignore = "needs the Julia 1 tokenizer in SACI_JULIA_MODEL_DIR"]
fn the_encoder_matches_the_reference_token_ids() {
    let dir = std::env::var("SACI_JULIA_MODEL_DIR").expect("set SACI_JULIA_MODEL_DIR");
    let tokenizer = std::fs::read_to_string(format!("{dir}/tokenizer.json")).unwrap();
    let limits = Limits {
        max_length: 1024,
        head_length: 256,
        strict: true,
    };
    let encoded = Encoder::new(&tokenizer, limits)
        .unwrap()
        .encode(
            Kind::Choice,
            "Which team should handle this?",
            &["Account support".to_owned(), "Billing".to_owned()],
            "The customer asks to reset their password.",
        )
        .unwrap();
    assert_eq!(
        encoded.ids,
        [
            2, 6241, 2872, 235_292, 12236, 2970, 1412, 6589, 736, 235_336, 1, 4, 10741, 2676, 4,
            97222, 1, 714, 6650, 19619, 577, 13846, 1024, 8003, 235_265, 1
        ]
    );
    assert_eq!(encoded.markers, [11, 14]);
    assert_eq!(encoded.qtype, 0);
}
