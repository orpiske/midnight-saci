//! The bounded HTTP tool for requests to the target environment.
//!
//! Requests are sent exactly as the model specified them, after `${NAME}`
//! expansion of environment parameters and secrets. Redirects are not
//! followed: a 3xx response is evidence like any other, and the model sends
//! a new request to the `Location` deliberately, so credentials are never
//! forwarded to another host implicitly. Any HTTP status is a response; only
//! a request that produced no response is a transport error.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use rig::rig_reqwest::reqwest::{self, Method, Url, redirect};
use saci_core::evidence::{HttpEvidence, HttpResponseEvidence};
use saci_core::redaction::{OutputCapture, Redactor, capture_text};
use saci_core::run::{ErrorKind, ExecutionError};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::credentials::Credentials;

/// Arguments of one HTTP tool call.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpRequest {
    /// Method, for example `GET`.
    pub method: String,
    /// Absolute `http` or `https` URL.
    pub url: String,
    /// Request headers.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Request body, sent as written.
    #[serde(default)]
    pub body: Option<String>,
}

impl HttpRequest {
    /// JSON schema of the tool arguments.
    #[must_use]
    pub fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "method": {"type": "string", "description": "HTTP method, for example GET or POST"},
                "url": {"type": "string", "description": "Absolute http or https URL; ${NAME} expands an environment parameter or secret"},
                "headers": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Request headers; values may use ${NAME}"},
                "body": {"type": "string", "description": "Request body, sent as written; may use ${NAME}"}
            },
            "required": ["method", "url"],
            "additionalProperties": false
        })
    }
}

/// One executed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRun {
    /// Redacted evidence.
    pub evidence: HttpEvidence,
    /// Wall-clock duration.
    pub duration: Duration,
}

/// The HTTP tool of one run.
#[derive(Debug, Clone)]
pub struct HttpTool {
    client: reqwest::Client,
    credentials: Credentials,
    output_limit: u64,
}

impl HttpTool {
    /// Builds the tool.
    ///
    /// # Errors
    ///
    /// Fails when the HTTP client cannot be built.
    pub fn new(credentials: Credentials, output_limit: u64) -> Result<Self, ExecutionError> {
        let client = reqwest::Client::builder()
            .redirect(redirect::Policy::none())
            .connect_timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| {
                ExecutionError::new(ErrorKind::Tool, crate::llm::client_build_error(&error))
            })?;
        Ok(Self {
            client,
            credentials,
            output_limit,
        })
    }

    /// Sends `request` within `deadline`, stopping early on `cancel`.
    ///
    /// # Errors
    ///
    /// Returns a message for the model when the arguments are invalid; the
    /// request is then not sent.
    pub async fn send(
        &self,
        request: &HttpRequest,
        deadline: Duration,
        cancel: &CancellationToken,
    ) -> Result<HttpRun, String> {
        let prepared = self.prepare(request)?;
        let mut evidence = self.request_evidence(&prepared);
        let started = Instant::now();
        let builder = self.builder(&prepared).timeout(deadline);
        let outcome = tokio::select! {
            outcome = self.exchange(builder) => outcome,
            () = cancel.cancelled() => Err("the request was cancelled".to_owned()),
        };
        let redactor = self.credentials.redactor();
        match outcome {
            Ok(mut response) => {
                redactor.redact_headers(&mut response.headers);
                evidence.response = Some(response);
            }
            Err(error) => evidence.transport_error = Some(redactor.redact(&error).into_owned()),
        }
        Ok(HttpRun {
            evidence,
            duration: started.elapsed(),
        })
    }

    fn prepare(&self, request: &HttpRequest) -> Result<Prepared, String> {
        let method = Method::from_bytes(request.method.trim().to_ascii_uppercase().as_bytes())
            .map_err(|_| format!("`{}` is not an HTTP method", request.method))?;
        let expanded = self.credentials.expand(&request.url);
        let url = Url::parse(&expanded).map_err(|error| format!("invalid URL: {error}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("only http and https URLs are supported".to_owned());
        }
        let mut headers = BTreeMap::new();
        for (name, value) in &request.headers {
            let value = self.credentials.expand(value);
            reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| format!("`{name}` is not a valid header name"))?;
            reqwest::header::HeaderValue::from_str(&value)
                .map_err(|_| format!("the value of header `{name}` is not valid"))?;
            headers.insert(name.clone(), value);
        }
        let body = request
            .body
            .as_deref()
            .map(|body| self.credentials.expand(body));
        Ok(Prepared {
            method,
            url,
            headers,
            body,
        })
    }

    fn request_evidence(&self, prepared: &Prepared) -> HttpEvidence {
        let redactor = self.credentials.redactor();
        let mut request_headers = prepared.headers.clone();
        redactor.redact_headers(&mut request_headers);
        HttpEvidence {
            method: prepared.method.to_string(),
            url: redactor.redact(prepared.url.as_str()).into_owned(),
            request_headers,
            request_body: prepared
                .body
                .as_deref()
                .map(|body| capture_text(body, self.output_limit, redactor)),
            response: None,
            transport_error: None,
        }
    }

    fn builder(&self, prepared: &Prepared) -> reqwest::RequestBuilder {
        let mut builder = self
            .client
            .request(prepared.method.clone(), prepared.url.clone());
        for (name, value) in &prepared.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = &prepared.body {
            builder = builder.body(body.clone());
        }
        builder
    }

    async fn exchange(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<HttpResponseEvidence, String> {
        let mut response = builder.send().await.map_err(|error| describe(&error))?;
        let status = response.status().as_u16();
        let headers = header_map(response.headers());
        let length = response.content_length();
        let redactor: &Redactor = self.credentials.redactor();
        let mut capture = OutputCapture::new(self.output_limit, redactor);
        let keep = self.output_limit.saturating_add(redactor.longest() as u64);
        while capture.total() <= keep {
            match response.chunk().await {
                Ok(Some(chunk)) => capture.push(&chunk),
                Ok(None) => break,
                Err(error) => {
                    return Err(format!(
                        "reading the response body failed: {}",
                        describe(&error)
                    ));
                }
            }
        }
        let mut body = capture.finish(redactor);
        if let Some(length) = length {
            body.total_bytes = body.total_bytes.max(length);
        }
        Ok(HttpResponseEvidence {
            status,
            headers,
            body,
        })
    }
}

struct Prepared {
    method: Method,
    url: Url,
    headers: BTreeMap<String, String>,
    body: Option<String>,
}

fn header_map(headers: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        map.entry(name.as_str().to_owned())
            .and_modify(|existing| {
                existing.push_str(", ");
                existing.push_str(&value);
            })
            .or_insert(value);
    }
    map
}

fn describe(error: &reqwest::Error) -> String {
    let mut chain = Vec::new();
    let mut source: Option<&dyn std::error::Error> = std::error::Error::source(error);
    while let Some(current) = source {
        chain.push(current.to_string());
        source = current.source();
    }
    let kind = if error.is_timeout() {
        "the request timed out"
    } else if error.is_connect() {
        "connection failed"
    } else if error.is_builder() {
        "the request could not be built"
    } else {
        "the request failed"
    };
    if chain.is_empty() {
        kind.to_owned()
    } else {
        format!("{kind}: {}", chain.join(": "))
    }
}
