// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Local IPC protocol, version 1: one JSON object per line over a Unix stream socket.
//!
//! Request:  `{"v":1,"id":"<correlation>","method":"submit","params":{...}}`
//! Response: `{"v":1,"id":"<correlation>","ok":true,"result":{...}}` or
//!           `{"v":1,"id":"<correlation>","ok":false,"error":{"code":"...","message":"..."}}`
//!
//! The requester's identity comes from the socket peer credentials, never from the message.

use serde::{Deserialize, Serialize};

use crate::request::Operation;

pub const PROTOCOL_VERSION: u32 = 1;
/// Largest accepted request line, bytes.
pub const MAX_REQUEST_BYTES: usize = 16 * 1024;
pub const MAX_AUDIT_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "method",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Method {
    Health,
    Capabilities,
    Context,
    Modules,
    /// The latest Zairenkai-Tuning telemetry document (`zairenkai.tuning` schema 1), as
    /// `ztune` printed it after Core validated its envelope.
    Tuning,
    Leases,
    Policy,
    Audit {
        #[serde(default = "default_audit_limit")]
        limit: usize,
    },
    Submit {
        operation: Operation,
        #[serde(default)]
        duration_ms: Option<u64>,
        #[serde(default)]
        dry_run: bool,
        #[serde(default)]
        confirmed: bool,
        #[serde(default)]
        reason: String,
    },
    /// Intelligence Engine only: a recommendation, never self-authorizing.
    Recommend {
        operation: Operation,
        confidence: f64,
        #[serde(default)]
        duration_ms: Option<u64>,
        #[serde(default)]
        reason: String,
    },
    Release {
        lease_id: u64,
    },
}

fn default_audit_limit() -> usize {
    50
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub v: u32,
    pub id: String,
    #[serde(flatten)]
    pub method: Method,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    UnsupportedVersion,
    TooLarge,
    RateLimited,
    Busy,
    PermissionDenied,
    NotFound,
    Timeout,
    ShuttingDown,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl Response {
    pub fn ok(id: &str, result: serde_json::Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: &str, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
            ok: false,
            result: None,
            error: Some(ErrorBody {
                code,
                message: message.into(),
            }),
        }
    }
}

/// Parse one request line. Errors carry the code to answer with.
pub fn parse_request(line: &str) -> Result<Request, (String, ErrorCode, String)> {
    if line.len() > MAX_REQUEST_BYTES {
        return Err((
            String::new(),
            ErrorCode::TooLarge,
            format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
        ));
    }
    let raw: serde_json::Value = serde_json::from_str(line).map_err(|e| {
        (
            String::new(),
            ErrorCode::BadRequest,
            format!("invalid JSON: {e}"),
        )
    })?;
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .chars()
        .take(64)
        .collect::<String>();
    match raw.get("v").and_then(|v| v.as_u64()) {
        Some(v) if v == PROTOCOL_VERSION as u64 => {}
        Some(v) => {
            return Err((
                id,
                ErrorCode::UnsupportedVersion,
                format!("protocol v{v} not supported; this Core speaks v{PROTOCOL_VERSION}"),
            ))
        }
        None => {
            return Err((
                id,
                ErrorCode::BadRequest,
                "missing protocol version \"v\"".into(),
            ))
        }
    }
    let req: Request = serde_json::from_value(raw)
        .map_err(|e| (id.clone(), ErrorCode::BadRequest, e.to_string()))?;
    if req.id.is_empty()
        || req.id.len() > 64
        || !req
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err((
            id,
            ErrorCode::BadRequest,
            "id must be 1..=64 chars of [A-Za-z0-9_-]".into(),
        ));
    }
    if let Method::Audit { limit } = req.method {
        if limit == 0 || limit > MAX_AUDIT_LIMIT {
            return Err((
                req.id,
                ErrorCode::BadRequest,
                format!("limit must be within 1..={MAX_AUDIT_LIMIT}"),
            ));
        }
    }
    Ok(req)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::FasProfile;

    #[test]
    fn parses_methods() {
        let r = parse_request(r#"{"v":1,"id":"a1","method":"submit","params":{"operation":{"op":"fas_set_profile","profile":"performance"},"duration_ms":60000}}"#).unwrap();
        assert_eq!(
            r.method,
            Method::Submit {
                operation: Operation::FasSetProfile {
                    profile: FasProfile::Performance
                },
                duration_ms: Some(60000),
                dry_run: false,
                confirmed: false,
                reason: String::new()
            }
        );
        assert_eq!(
            parse_request(r#"{"v":1,"id":"h","method":"health"}"#)
                .unwrap()
                .method,
            Method::Health
        );
        assert_eq!(
            parse_request(r#"{"v":1,"id":"h","method":"audit","params":{}}"#)
                .unwrap()
                .method,
            Method::Audit { limit: 50 }
        );
    }

    #[test]
    fn rejects_bad_input() {
        let code = |s: &str| parse_request(s).unwrap_err().1;
        assert_eq!(code("{"), ErrorCode::BadRequest);
        assert_eq!(
            code(r#"{"v":2,"id":"x","method":"health"}"#),
            ErrorCode::UnsupportedVersion
        );
        assert_eq!(
            code(r#"{"id":"x","method":"health"}"#),
            ErrorCode::BadRequest
        );
        assert_eq!(
            code(r#"{"v":1,"id":"x","method":"exec","params":{"cmd":"reboot"}}"#),
            ErrorCode::BadRequest
        );
        assert_eq!(
            code(
                r#"{"v":1,"id":"x","method":"submit","params":{"operation":{"op":"shell","cmd":"id"}}}"#
            ),
            ErrorCode::BadRequest
        );
        assert_eq!(
            code(r#"{"v":1,"id":"x y","method":"health"}"#),
            ErrorCode::BadRequest
        );
        assert_eq!(
            code(r#"{"v":1,"id":"x","method":"audit","params":{"limit":100000}}"#),
            ErrorCode::BadRequest
        );
        assert_eq!(
            code(r#"{"v":1,"id":"x","method":"health","extra":1}"#),
            ErrorCode::BadRequest
        );
        assert_eq!(
            code(&format!(
                r#"{{"v":1,"id":"x","method":"submit","params":{{"reason":"{}"}}}}"#,
                "a".repeat(MAX_REQUEST_BYTES)
            )),
            ErrorCode::TooLarge
        );
    }
}
