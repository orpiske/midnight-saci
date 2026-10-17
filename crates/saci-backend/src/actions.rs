//! The outcome action dispatcher (`docs/actions.md`): a backend task, apart
//! from the API and from workers, that calls the configured MCP tools of
//! queued actions.
//!
//! It holds the only connections to action servers and the only copies of
//! `actions.secrets`. It sees an action's recorded arguments and nothing
//! else of the run. An action's delivery never changes the run's report.
//!
//! A call is never retried blindly. When the call was sent but no answer
//! was received (a timeout, a broken connection, or a backend restart),
//! the delivery is unknown. An action with an idempotency key is then
//! replayed with the same key, up to [`MAX_ATTEMPTS`] attempts; any other
//! action stays `unknown` until a person reconciles or redispatches it.

use std::collections::BTreeMap;
use std::time::Duration;

use saci_core::action::{ActionRecord, ActionState};
use saci_core::config::{ActionsConfig, Config};
use saci_core::redaction::Redactor;
use saci_core::spec::{McpServerSpec, SecretRef};
use saci_mcp::rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::Value;

use crate::api::Backend;
use crate::store::Delivery;

/// Longest result text kept for an action.
const RESULT_LIMIT: usize = 64 * 1024;
/// How often queued actions are looked for without a change notification.
const POLL: Duration = Duration::from_secs(5);
/// Attempts after which a keyed action whose answers keep getting lost is
/// left `unknown`.
pub const MAX_ATTEMPTS: u32 = 3;

/// What action servers receive: `PATH` for stdio servers, and the values of
/// `actions.secrets` for `${NAME}`.
struct ActionCredentials {
    environment: BTreeMap<String, String>,
    values: BTreeMap<String, String>,
    redactor: Redactor,
}

impl saci_mcp::Projection for ActionCredentials {
    fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }

    fn expand(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for (name, value) in &self.values {
            out = out.replace(&format!("${{{name}}}"), value);
        }
        out
    }
}

fn credentials(
    actions: &ActionsConfig,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<ActionCredentials, String> {
    let mut values = BTreeMap::new();
    let mut redactor = Redactor::new();
    for (name, secret) in &actions.secrets {
        let SecretRef::Env { env } = secret else {
            return Err(format!("actions.secrets.{name} must use {{ env: VAR }}"));
        };
        let value = lookup(env)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!("secret `{name}` needs backend variable `{env}`, which is not set")
            })?;
        redactor.add(name, &value);
        values.insert(name.clone(), value);
    }
    let environment = lookup("PATH")
        .map(|path| BTreeMap::from([("PATH".to_owned(), path)]))
        .unwrap_or_default();
    Ok(ActionCredentials {
        environment,
        values,
        redactor,
    })
}

/// Reads a backend environment variable.
pub type Lookup = std::sync::Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Dispatches queued actions until the process ends, reading
/// `actions.secrets` from the backend's environment.
pub async fn run(backend: Backend) {
    Box::pin(run_with(
        backend,
        std::sync::Arc::new(|name| std::env::var(name).ok()),
    ))
    .await;
}

/// [`run`], reading backend variables with `lookup`.
pub async fn run_with(backend: Backend, lookup: Lookup) {
    let mut changes = backend.subscribe();
    recover(&backend).await;
    loop {
        loop {
            match backend.store.claim_action().await {
                Ok(Some(action)) => {
                    Box::pin(dispatch_and_record(&backend, action, &lookup)).await;
                }
                Ok(None) => break,
                Err(error) => {
                    tracing::error!(%error, "cannot read queued actions");
                    break;
                }
            }
        }
        let _ = tokio::time::timeout(POLL, changes.changed()).await;
    }
}

/// Actions a stopped backend left `executing` may or may not have reached
/// their server: their delivery is unknown, and only keyed ones are
/// replayed.
async fn recover(backend: &Backend) {
    let interrupted = match backend.store.executing_actions().await {
        Ok(actions) => actions,
        Err(error) => {
            tracing::error!(%error, "cannot read interrupted actions");
            return;
        }
    };
    for action in interrupted {
        let delivery = uncertain("the backend stopped while dispatching this action");
        record(backend, &action, settle(&action, delivery)).await;
    }
}

async fn dispatch_and_record(backend: &Backend, action: ActionRecord, lookup: &Lookup) {
    tracing::info!(run = %action.run_id, action = %action.name, "dispatching action");
    let delivery = Box::pin(dispatch(&backend.config, &action, &|name: &str| {
        lookup(name)
    }))
    .await;
    record(
        backend,
        &action,
        settle(&action, earlier(&action, delivery)),
    )
    .await;
}

/// A later attempt that failed before sending anything does not settle the
/// action: an earlier attempt may have reached the server.
fn earlier(action: &ActionRecord, delivery: Delivery) -> Delivery {
    let unsent = delivery.state == ActionState::Error && delivery.result.is_none();
    if !unsent || action.attempts <= 1 {
        return delivery;
    }
    let reason = delivery.error.unwrap_or_default();
    uncertain(&format!(
        "attempt {} was not sent ({reason}), and an earlier attempt may have reached the server",
        action.attempts
    ))
}

/// A delivery whose call may have reached the server.
fn uncertain(reason: &str) -> Delivery {
    Delivery {
        state: ActionState::Unknown,
        result: None,
        error: Some(format!(
            "{reason}; whether the server acted on the call is unknown"
        )),
    }
}

/// Decides what an unknown delivery leads to: a replay with the action's
/// idempotency key while attempts remain, otherwise `unknown`, waiting for
/// a person.
fn settle(action: &ActionRecord, mut delivery: Delivery) -> Delivery {
    if delivery.state != ActionState::Unknown {
        return delivery;
    }
    let reason = delivery.error.take().unwrap_or_default();
    delivery.error = Some(match &action.idempotency_key {
        Some(_) if action.attempts < MAX_ATTEMPTS => {
            delivery.state = ActionState::Queued;
            format!("{reason}; replaying it with its idempotency key")
        }
        Some(_) => format!(
            "{reason}; {} attempts were made with its idempotency key; reconcile or redispatch it",
            action.attempts
        ),
        None => format!(
            "{reason}; it has no idempotency key, so it is not sent again: reconcile or redispatch it"
        ),
    });
    delivery
}

async fn record(backend: &Backend, action: &ActionRecord, delivery: Delivery) {
    let state = delivery.state;
    match backend
        .store
        .finish_action(&action.run_id, &action.name, delivery)
        .await
    {
        Ok(_) => {
            tracing::info!(run = %action.run_id, action = %action.name, %state, "action finished");
        }
        Err(error) => {
            tracing::error!(run = %action.run_id, action = %action.name, %error, "cannot record the action's delivery");
        }
    }
    backend.notify();
}

fn failed(message: impl Into<String>) -> Delivery {
    Delivery {
        state: ActionState::Error,
        result: None,
        error: Some(message.into()),
    }
}

/// Connects to the action's server, checks the recorded arguments against
/// the tool's schema, and calls it once.
pub async fn dispatch(
    config: &Config,
    action: &ActionRecord,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Delivery {
    let Some(transport) = config.actions.servers.get(&action.server) else {
        return failed(format!(
            "action server `{}` is no longer configured",
            action.server
        ));
    };
    let credentials = match credentials(&config.actions, lookup) {
        Ok(credentials) => credentials,
        Err(message) => return failed(message),
    };
    let timeout = Duration::from_secs(config.default_limits().operation_timeout_secs);
    let spec = McpServerSpec {
        transport: transport.clone(),
        tools: Some(vec![action.tool.clone()]),
    };
    let client = saci_mcp::Client {
        name: "saci-backend",
        place: "backend",
        projection: &credentials,
        timeout,
    };
    let server = match Box::pin(saci_mcp::connect(&action.server, &spec, client)).await {
        Ok(server) => server,
        Err(error) => return failed(credentials.redactor.redact(&error.message)),
    };
    let delivery = Box::pin(call(&server, action, timeout, &credentials.redactor)).await;
    server.shutdown().await;
    delivery
}

async fn call(
    server: &saci_mcp::McpServer,
    action: &ActionRecord,
    timeout: Duration,
    redactor: &Redactor,
) -> Delivery {
    let Value::Object(arguments) = action.arguments.clone() else {
        return failed("the recorded arguments are not an object");
    };
    let Some(tool) = server.tools().first() else {
        return failed(format!("the server does not offer `{}`", action.tool));
    };
    if let Err(problems) = saci_mcp::check_arguments(tool, &arguments) {
        return failed(format!(
            "the arguments do not match the input schema of `{}`: {problems}; nothing was sent",
            action.tool
        ));
    }
    let request = CallToolRequestParams::new(tool.name.clone()).with_arguments(arguments);
    let redact = |text: &str| bounded(&redactor.redact(text));
    match tokio::time::timeout(timeout, Box::pin(server.peer().call_tool(request))).await {
        Ok(Ok(result)) => answered(redact(&saci_mcp::result_text(&result)), &result),
        Ok(Err(error)) => uncertain(&redact(&format!("the call failed: {error}"))),
        Err(_) => uncertain(&format!("no answer within {}s", timeout.as_secs())),
    }
}

/// The delivery of a call the server answered.
fn answered(text: String, result: &CallToolResult) -> Delivery {
    if result.is_error.unwrap_or(false) {
        Delivery {
            state: ActionState::Error,
            error: Some(format!("the tool reported an error: {text}")),
            result: Some(text),
        }
    } else {
        Delivery {
            state: ActionState::Completed,
            result: Some(text),
            error: None,
        }
    }
}

fn bounded(text: &str) -> String {
    if text.len() <= RESULT_LIMIT {
        return text.to_owned();
    }
    let mut end = RESULT_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[result truncated]", &text[..end])
}
