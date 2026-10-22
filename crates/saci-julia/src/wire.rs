//! The evaluation request and answer format.
//!
//! It is the TypeSafe System One wire format, so the Saci judge can use one
//! client for both providers:
//!
//! ```json
//! {"model": "julia-1", "state": {...}, "questions": {"q1": {"type": "noul", "instructions": "..."}}}
//! ```
//!
//! The answer rules follow `julia/typed.py` in SupersonicLabs/Julia-1.

use serde_json::{Map, Value, json};

use crate::encode::{Kind, MAX_OPTIONS, MIN_OPTIONS};

/// One question, ready for the encoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub kind: Kind,
    pub instructions: String,
    /// Answer keys: criteria IDs for choice, `"0".."n-1"` for score,
    /// `"false"` and `"true"` for noul.
    pub keys: Vec<String>,
    /// Option text sent to the model, in key order.
    pub options: Vec<String>,
}

/// A parsed evaluation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The state as text: a JSON string as is, other values as compact JSON.
    pub state: String,
    pub questions: Vec<Question>,
}

/// Text for the model: a string as is, any other value as compact JSON.
fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn non_empty(value: &Value, what: &str) -> Result<String, String> {
    let text = text(value);
    if text.trim().is_empty() || value.is_null() {
        return Err(format!("{what} must not be empty"));
    }
    Ok(text)
}

/// Parses an evaluation request.
///
/// # Errors
///
/// A message that names the first problem.
pub fn parse(value: &Value) -> Result<Request, String> {
    let object = value
        .as_object()
        .ok_or("the request must be a JSON object")?;
    let state = object.get("state").ok_or("state is required")?;
    let state = non_empty(state, "state")?;
    let questions = match object.get("questions") {
        Some(Value::Object(questions)) if !questions.is_empty() => questions,
        _ => return Err("questions must be a non-empty object".into()),
    };
    let questions = questions
        .iter()
        .map(|(id, question)| {
            question_of(id, question).map_err(|error| format!("question {id:?}: {error}"))
        })
        .collect::<Result<_, _>>()?;
    Ok(Request { state, questions })
}

fn question_of(id: &str, value: &Value) -> Result<Question, String> {
    let question = value.as_object().ok_or("must be an object")?;
    let kind = question
        .get("type")
        .and_then(Value::as_str)
        .and_then(Kind::parse)
        .ok_or("type must be choice, score or noul")?;
    let instructions = non_empty(
        question.get("instructions").unwrap_or(&Value::Null),
        "instructions",
    )?;
    let criteria = question.get("criteria").filter(|value| !value.is_null());
    let (keys, options) = criteria_of(kind, criteria)?;
    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&options.len()) {
        return Err(format!("needs {MIN_OPTIONS} to {MAX_OPTIONS} criteria"));
    }
    Ok(Question {
        id: id.to_owned(),
        kind,
        instructions,
        keys,
        options,
    })
}

/// The answer keys and the option text of a question.
fn criteria_of(kind: Kind, criteria: Option<&Value>) -> Result<(Vec<String>, Vec<String>), String> {
    match (kind, criteria) {
        (Kind::Choice, Some(Value::Object(criteria))) => choice_criteria(criteria),
        (Kind::Choice, _) => Err("choice criteria must be an object".into()),
        (Kind::Score, Some(Value::Array(levels))) => {
            let options = levels
                .iter()
                .enumerate()
                .map(|(index, level)| non_empty(level, &format!("score level {index}")))
                .collect::<Result<Vec<_>, _>>()?;
            let keys = (0..options.len()).map(|index| index.to_string()).collect();
            Ok((keys, options))
        }
        (Kind::Score, _) => Err("score criteria must be an ordered list".into()),
        (Kind::Noul, None) => Ok((noul_keys(), noul_keys())),
        (Kind::Noul, Some(Value::Object(criteria)))
            if criteria.len() == 2
                && criteria.contains_key("false")
                && criteria.contains_key("true") =>
        {
            let side = |key: &str| non_empty(&criteria[key], &format!("criterion {key:?}"));
            Ok((noul_keys(), vec![side("false")?, side("true")?]))
        }
        (Kind::Noul, _) => {
            Err("noul criteria must be omitted or have exactly the keys false and true".into())
        }
    }
}

/// Choice criteria in request order. A criterion without a description is
/// described by its ID.
fn choice_criteria(criteria: &Map<String, Value>) -> Result<(Vec<String>, Vec<String>), String> {
    let options = criteria
        .iter()
        .map(|(key, description)| match description {
            Value::Null => Ok(key.clone()),
            other => non_empty(other, &format!("criterion {key:?}")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((criteria.keys().cloned().collect(), options))
}

fn noul_keys() -> Vec<String> {
    vec!["false".into(), "true".into()]
}

/// Softmax with the maximum subtracted first, for numeric stability.
#[must_use]
pub fn softmax(logits: &[f32]) -> Vec<f64> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f64> = logits.iter().map(|&l| f64::from(l - max).exp()).collect();
    let sum: f64 = exps.iter().sum();
    exps.into_iter().map(|e| e / sum).collect()
}

/// The answer to one question from its raw logits.
///
/// # Errors
///
/// Fails when the model returned the wrong number of logits, or one that is
/// not finite.
pub fn answer(question: &Question, logits: &[f32]) -> Result<Value, String> {
    check_logits(question, logits)?;
    let probabilities = softmax(logits);
    let (best, confidence) = top(&probabilities);
    let by_key: Map<String, Value> = question
        .keys
        .iter()
        .cloned()
        .zip(probabilities.iter().map(|&p| json!(p)))
        .collect();
    Ok(match question.kind {
        Kind::Choice => json!({
            "type": "choice",
            "choice": question.keys[best],
            "probabilities": by_key,
            "confidence": confidence,
        }),
        Kind::Score => json!({
            "type": "score",
            "score": expected_index(&probabilities),
            "probabilities": by_key,
            "legend": legend(question),
            "confidence": confidence,
        }),
        Kind::Noul => json!({"type": "noul", "noul": probabilities[1]}),
    })
}

/// The index and value of the largest probability.
fn top(probabilities: &[f64]) -> (usize, f64) {
    probabilities
        .iter()
        .copied()
        .enumerate()
        .fold(
            (0, f64::NEG_INFINITY),
            |top, (i, p)| if p > top.1 { (i, p) } else { top },
        )
}

/// The text of each score level, by key.
fn legend(question: &Question) -> Map<String, Value> {
    question
        .keys
        .iter()
        .cloned()
        .zip(question.options.iter().map(|level| json!(level)))
        .collect()
}

fn check_logits(question: &Question, logits: &[f32]) -> Result<(), String> {
    if logits.len() != question.keys.len() {
        return Err(format!(
            "the model returned {} scores for {} options",
            logits.len(),
            question.keys.len()
        ));
    }
    if logits.iter().any(|logit| !logit.is_finite()) {
        return Err("the model returned a score that is not finite".into());
    }
    Ok(())
}

/// The expected rubric position, from 0.
fn expected_index(probabilities: &[f64]) -> f64 {
    probabilities
        .iter()
        .zip(0u32..)
        .map(|(p, i)| f64::from(i) * p)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_text(text: &str) -> Result<Request, String> {
        parse(&serde_json::from_str(text).unwrap())
    }

    #[test]
    fn the_judge_request_parses_into_noul_questions() {
        let request = parse_text(
            r#"{"model": "julia-1", "state": {"step": "Check", "evidence": ["ok"]},
                "questions": {"criterion_0": {"type": "noul", "instructions": "Holds?",
                    "criteria": {"true": "It holds", "false": "It does not"}}}}"#,
        )
        .unwrap();
        assert_eq!(request.state, r#"{"step":"Check","evidence":["ok"]}"#);
        let question = &request.questions[0];
        assert_eq!(question.id, "criterion_0");
        assert_eq!(question.keys, ["false", "true"]);
        assert_eq!(question.options, ["It does not", "It holds"]);
    }

    #[test]
    fn choice_and_score_criteria_keep_their_order() {
        let request = parse_text(
            r#"{"state": "s", "questions": {
                "team": {"type": "choice", "instructions": "Who?", "criteria": {"z": "Zed", "a": null}},
                "level": {"type": "score", "instructions": "How?", "criteria": ["low", "high"]},
                "plain": {"type": "noul", "instructions": "True?"}}}"#,
        )
        .unwrap();
        let [team, level, plain] = &request.questions[..] else {
            panic!("three questions expected");
        };
        assert_eq!(
            (team.keys.clone(), team.options.clone()),
            (vec!["z".into(), "a".into()], vec!["Zed".into(), "a".into()])
        );
        assert_eq!(level.keys, ["0", "1"]);
        assert_eq!(plain.options, ["false", "true"]);
    }

    /// Invalid requests and the message each one gets.
    const BAD_REQUESTS: [(&str, &str); 8] = [
        (r#"[]"#, "the request must be a JSON object"),
        (r#"{"questions": {}}"#, "state is required"),
        (
            r#"{"state": "", "questions": {}}"#,
            "state must not be empty",
        ),
        (
            r#"{"state": "s", "questions": {}}"#,
            "questions must be a non-empty object",
        ),
        (
            r#"{"state": "s", "questions": {"q": {"type": "x", "instructions": "i"}}}"#,
            "question \"q\": type must be choice, score or noul",
        ),
        (
            r#"{"state": "s", "questions": {"q": {"type": "noul"}}}"#,
            "question \"q\": instructions must not be empty",
        ),
        (
            r#"{"state": "s", "questions": {"q": {"type": "choice", "instructions": "i", "criteria": {"a": "A"}}}}"#,
            "question \"q\": needs 2 to 20 criteria",
        ),
        (
            r#"{"state": "s", "questions": {"q": {"type": "noul", "instructions": "i", "criteria": {"yes": "Y", "no": "N"}}}}"#,
            "question \"q\": noul criteria must be omitted or have exactly the keys false and true",
        ),
    ];

    #[test]
    fn bad_requests_name_the_problem() {
        for (text, expected) in BAD_REQUESTS {
            assert_eq!(parse_text(text).unwrap_err(), expected, "{text}");
        }
    }

    fn question(kind: Kind, keys: &[&str]) -> Question {
        Question {
            id: "q".into(),
            kind,
            instructions: "i".into(),
            keys: keys.iter().map(|k| (*k).into()).collect(),
            options: keys.iter().map(|k| format!("level {k}")).collect(),
        }
    }

    #[test]
    fn answers_use_the_typesafe_shapes() {
        let noul = answer(&question(Kind::Noul, &["false", "true"]), &[0.0, 0.0]).unwrap();
        assert_eq!(noul, json!({"type": "noul", "noul": 0.5}));

        let choice = answer(&question(Kind::Choice, &["a", "b", "c"]), &[0.1, 3.0, -1.0]).unwrap();
        assert_eq!(choice["choice"], "b");
        assert!(choice["confidence"].as_f64().unwrap() > 0.9);

        let score = answer(&question(Kind::Score, &["0", "1", "2"]), &[0.0, 0.0, 0.0]).unwrap();
        assert!((score["score"].as_f64().unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(score["legend"]["2"], "level 2");
    }

    #[test]
    fn softmax_is_stable_and_answers_reject_bad_logits() {
        let p = softmax(&[1000.0, 1000.0]);
        assert!((p[0] - 0.5).abs() < 1e-12);
        let q = question(Kind::Choice, &["a", "b"]);
        assert!(answer(&q, &[1.0]).is_err());
        assert!(answer(&q, &[1.0, f32::NAN]).is_err());
    }
}
