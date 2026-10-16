//! The account's models — what the Claude Code view's model chooser lists.
//!
//! `GET /v1/models` on the Anthropic API, spent through [`crate::web_history::api_get`],
//! the one place this crate spends the OAuth credential. So the token is read where
//! the CLI keeps it (the credentials file, or the Keychain on macOS, following
//! `CLAUDE_CONFIG_DIR`), a token past its life is renewed by the CLI rather than by us,
//! and the token never crosses JNI: Java gets each model's id and display name, and
//! picks the newest of each family itself (`ModelCatalog`).

use crate::web_history::ApiError;
use serde_json::{json, Value};

/// The list call. One page holds the whole list: the API lists a few dozen models.
const MODELS_PATH: &str = "/v1/models?limit=100";

/// The account's models as `{"ok":true,"data":[{"id","display_name"},…]}`, or
/// `{"ok":false,"state","error"}` saying why there are none: `signed-out` (no claude.ai
/// login: an API key, or none), `expired` (refused even after the CLI renewed the
/// token) or `error` (the network, the API, an answer that is not a model list).
///
/// **Blocking** — the credentials file, or `security` on macOS, one HTTPS round trip,
/// and on a stale credential a short-lived `claude` process. Off the UI thread.
pub fn fetch(claude_cmd: &str) -> String {
    answer(crate::web_history::api_get(claude_cmd, MODELS_PATH))
}

/// What Java is told about one call.
fn answer(result: Result<String, ApiError>) -> String {
    let json = match result {
        Ok(body) => match models_of(&body) {
            Some(data) => {
                if crate::is_debug() {
                    crate::elog!("[models] {} models", data.len());
                }
                return json!({ "ok": true, "data": data }).to_string();
            }
            None => failure("error", "the answer was not a model list"),
        },
        Err(ApiError::SignedOut) => failure("signed-out", "no claude.ai login"),
        Err(ApiError::Expired) => failure("expired", "refused after the CLI renewed the login"),
        Err(ApiError::Failed(reason)) => failure("error", &reason),
    };
    if crate::is_debug() {
        crate::elog!("[models] no list: {} ({})", json["state"], json["error"]);
    }
    json.to_string()
}

/// Each model's `id` and `display_name` (when it has one), in the API's order; `None`
/// when the body is not a model list.
fn models_of(body: &str) -> Option<Vec<Value>> {
    let v: Value = serde_json::from_str(body).ok()?;
    let data = v.get("data")?.as_array()?;
    Some(
        data.iter()
            .filter_map(|m| {
                let mut model = json!({ "id": m.get("id")?.as_str()? });
                if let Some(name) = m.get("display_name").and_then(Value::as_str) {
                    model["display_name"] = json!(name);
                }
                Some(model)
            })
            .collect(),
    )
}

/// The answer for a call that produced no list.
fn failure(state: &str, error: &str) -> Value {
    json!({ "ok": false, "state": state, "error": error })
}


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(s: &str) -> Value {
        serde_json::from_str(s).expect("the answer is JSON")
    }

    #[test]
    fn lists_each_model_by_id_and_display_name_only() {
        let body = r#"{"data":[
            {"type":"model","id":"claude-opus-5-5","display_name":"Claude Opus 5.5","created_at":"2026-09-01T00:00:00Z"},
            {"type":"model","id":"claude-haiku-5-5","display_name":"Claude Haiku 5.5","created_at":"2026-09-01T00:00:00Z"}],
            "has_more":false,"first_id":"claude-opus-5-5","last_id":"claude-haiku-5-5"}"#;
        assert_eq!(
            parsed(&answer(Ok(body.into()))),
            json!({"ok": true, "data": [
                {"id": "claude-opus-5-5", "display_name": "Claude Opus 5.5"},
                {"id": "claude-haiku-5-5", "display_name": "Claude Haiku 5.5"}]})
        );
    }

    #[test]
    fn a_model_without_a_display_name_keeps_its_id() {
        let body = r#"{"data":[{"id":"claude-fable-5-1"},{"display_name":"no id"}]}"#;
        assert_eq!(
            parsed(&answer(Ok(body.into()))),
            json!({"ok": true, "data": [{"id": "claude-fable-5-1"}]})
        );
    }

    #[test]
    fn a_body_that_is_not_a_model_list_is_an_error() {
        for body in ["not json", r#"{"type":"error"}"#, r#"{"data":{}}"#] {
            let a = parsed(&answer(Ok(body.into())));
            assert_eq!(a["ok"], json!(false), "{body}");
            assert_eq!(a["state"], json!("error"), "{body}");
        }
    }

    #[test]
    fn says_why_there_is_no_list() {
        assert_eq!(parsed(&answer(Err(ApiError::SignedOut)))["state"], json!("signed-out"));
        assert_eq!(parsed(&answer(Err(ApiError::Expired)))["state"], json!("expired"));
        let failed = parsed(&answer(Err(ApiError::Failed("HTTP 500".into()))));
        assert_eq!(failed, json!({"ok": false, "state": "error", "error": "HTTP 500"}));
    }

    #[test]
    fn models_of_reads_the_api_order() {
        let ids: Vec<Value> = models_of(r#"{"data":[{"id":"b"},{"id":"a"}]}"#)
            .expect("a list")
            .into_iter()
            .map(|m| m["id"].clone())
            .collect();
        assert_eq!(ids, vec![json!("b"), json!("a")]);
    }
}
