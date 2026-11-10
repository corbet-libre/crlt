//! The Hrana 3 HTTP pipeline subset: execute, autocommit status and close.
//! Wire types and value encoding come from the official libsql-hrana crate.

use std::time::Duration;

use libsql::params::Params;
use libsql_hrana::proto::{self, StreamRequest, StreamResponse, StreamResult};
use reqwest::header::HeaderValue;
use tokio::sync::Mutex;

use crate::{
    Error, Result, Value,
    transport::{Sender, failure, validate_url},
};

pub(crate) struct Remote {
    sender: Sender,
    auth: HeaderValue,
    timeout: Duration,
    state: Mutex<State>,
}

struct State {
    url: url::Url,
    baton: Option<String>,
    autocommit: bool,
    broken: bool,
}

impl Remote {
    pub(crate) fn new(url: &str, token: &str, sender: Sender, timeout: Duration) -> Result<Self> {
        let url = url
            .strip_prefix("libsql://")
            .map(|rest| format!("https://{rest}"))
            .unwrap_or_else(|| url.to_owned());
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| Error::Config("invalid authentication token"))?;
        auth.set_sensitive(true);
        Ok(Self {
            sender,
            auth,
            timeout,
            state: Mutex::new(State {
                url: pipeline_url(&url)?,
                baton: None,
                autocommit: true,
                broken: false,
            }),
        })
    }

    pub(crate) async fn reusable(&self) -> bool {
        let state = self.state.lock().await;
        !state.broken && state.autocommit
    }

    pub(crate) async fn execute(
        &self,
        sql: &str,
        params: Params,
        want_rows: bool,
    ) -> Result<proto::StmtResult> {
        let mut stmt = proto::Stmt::new(sql, want_rows);
        match params {
            Params::None => {}
            Params::Positional(values) => {
                stmt.args = values
                    .into_iter()
                    .map(encode_value)
                    .collect::<Result<_>>()?;
            }
            Params::Named(values) => {
                for (name, value) in values {
                    stmt.bind_named(name, encode_value(value)?);
                }
            }
        }
        let mut state = self.state.lock().await;
        if state.broken {
            return Err(failure("Hrana stream is unusable"));
        }
        let request = proto::PipelineReqBody {
            baton: state.baton.clone(),
            requests: vec![
                StreamRequest::Execute(proto::ExecuteStreamReq { stmt }),
                StreamRequest::GetAutocommit(proto::GetAutocommitStreamReq {}),
            ],
        };
        // Cancellation, malformed replies and transport errors must never cause
        // the next statement to silently open a replacement transaction.
        state.broken = true;
        let response = self.sender.send(&state.url, &self.auth, &request).await?;
        state.baton = response.baton;
        if let Some(base_url) = response.base_url {
            state.url = pipeline_url(&base_url)?;
        }
        if state.baton.as_ref().is_none_or(|baton| baton.is_empty()) || response.results.len() != 2
        {
            return Err(failure("Hrana stream closed or response count changed"));
        }
        let mut results = response.results.into_iter();
        let statement = results.next().expect("two results");
        match results.next().expect("two results") {
            StreamResult::Ok {
                response: StreamResponse::GetAutocommit(result),
            } => {
                state.autocommit = result.is_autocommit;
            }
            _ => return Err(failure("missing Hrana transaction status")),
        }
        match statement {
            StreamResult::Ok {
                response: StreamResponse::Execute(result),
            } => {
                state.broken = false;
                Ok(result.result)
            }
            StreamResult::Error { .. } => {
                // Keep a valid baton so explicit rollback remains possible. SQL
                // failure still poisons the public transaction at the facade.
                state.broken = false;
                Err(failure("remote SQL statement failed"))
            }
            _ => Err(failure("unexpected Hrana statement response")),
        }
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        let state = self.state.get_mut();
        let Some(baton) = state.baton.take() else {
            return;
        };
        // Close rolls back the stream. With a lost response, the last known baton
        // may be stale; cleanup is bounded/best effort, then sqld expiry applies.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let sender = self.sender.clone();
            let auth = self.auth.clone();
            let url = state.url.clone();
            let timeout = self.timeout;
            runtime.spawn(async move {
                let request = proto::PipelineReqBody {
                    baton: Some(baton),
                    requests: vec![StreamRequest::Close(proto::CloseStreamReq {})],
                };
                let _ = tokio::time::timeout(timeout, sender.send(&url, &auth, &request)).await;
            });
        }
    }
}

fn pipeline_url(base: &str) -> Result<url::Url> {
    let mut url = url::Url::parse(base).map_err(|_| failure("invalid Hrana base URL"))?;
    validate_url(&url)?;
    url.set_path(&format!("{}/v3/pipeline", url.path().trim_end_matches('/')));
    Ok(url)
}

fn encode_value(value: Value) -> Result<proto::Value> {
    Ok(match value {
        Value::Null => proto::Value::Null,
        Value::Integer(value) => proto::Value::Integer { value },
        Value::Real(value) if value.is_finite() => proto::Value::Float { value },
        Value::Real(_) => return Err(failure("non-finite Hrana float")),
        Value::Text(value) => proto::Value::Text {
            value: value.into(),
        },
        Value::Blob(value) => proto::Value::Blob {
            value: value.into(),
        },
    })
}

pub(crate) fn decode_value(value: proto::Value) -> Result<Value> {
    Ok(match value {
        proto::Value::Null => Value::Null,
        proto::Value::Integer { value } => Value::Integer(value),
        proto::Value::Float { value } if value.is_finite() => Value::Real(value),
        proto::Value::Text { value } => Value::Text(value.to_string()),
        proto::Value::Blob { value } => Value::Blob(value.to_vec()),
        _ => return Err(failure("invalid Hrana value")),
    })
}

#[cfg(test)]
#[path = "tests/remote.rs"]
mod tests;
