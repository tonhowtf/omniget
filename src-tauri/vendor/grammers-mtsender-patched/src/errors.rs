// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
use grammers_mtproto::{authentication, mtp, transport};
use grammers_tl_types as tl;
use std::fmt;
use std::io;

/// This error occurs when reading from the network fails.
#[derive(Debug)]
pub enum ReadError {
    /// Standard I/O error.
    Io(io::Error),
    /// Error propagated from the underlying [`transport`].
    Transport(transport::Error),
    /// Error propagated from attempting to deserialize an invalid [`tl::Deserializable`].
    Deserialize(mtp::DeserializeError),
}

impl std::error::Error for ReadError {}

impl Clone for ReadError {
    fn clone(&self) -> Self {
        match self {
            Self::Io(e) => Self::Io(
                e.raw_os_error()
                    .map(io::Error::from_raw_os_error)
                    .unwrap_or_else(|| io::Error::new(e.kind(), e.to_string())),
            ),
            Self::Transport(e) => Self::Transport(e.clone()),
            Self::Deserialize(e) => Self::Deserialize(e.clone()),
        }
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "read error, IO failed: {err}"),
            Self::Transport(err) => write!(f, "read error, transport-level: {err}"),
            Self::Deserialize(err) => write!(f, "read error, bad response: {err}"),
        }
    }
}

impl From<io::Error> for ReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<transport::Error> for ReadError {
    fn from(error: transport::Error) -> Self {
        Self::Transport(error)
    }
}

impl From<mtp::DeserializeError> for ReadError {
    fn from(error: mtp::DeserializeError) -> Self {
        Self::Deserialize(error)
    }
}

impl From<tl::deserialize::Error> for ReadError {
    fn from(error: tl::deserialize::Error) -> Self {
        Self::Deserialize(error.into())
    }
}

/// The error type reported by the server when a request is misused.
///
/// These are returned when Telegram respond to an RPC with [`tl::types::RpcError`].
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    /// A numerical value similar to [HTTP response status codes](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Status).
    pub code: i32,

    /// The ASCII error name, normally in screaming snake case.
    ///
    /// Digit words are removed from the name and put in the [`RpcError::value`] instead.
    /// ```
    /// use grammers_mtsender::RpcError;
    /// let rpc_error = RpcError::from(grammers_tl_types::types::RpcError {
    ///         error_code: 500, error_message: "INTERDC_2_CALL_ERROR".into() });
    /// assert_eq!(rpc_error.name, "INTERDC_CALL_ERROR");
    /// assert_eq!(rpc_error.value, Some(2));
    /// ```
    pub name: String,

    /// If the error contained an additional integer value, it will be present here and removed from the [`RpcError::name`].
    /// ```
    /// use grammers_mtsender::RpcError;
    /// let rpc_error = RpcError::from(grammers_tl_types::types::RpcError {
    ///         error_code: 420, error_message: "FLOOD_WAIT_31".into() });
    /// assert_eq!(rpc_error.name, "FLOOD_WAIT");
    /// assert_eq!(rpc_error.value, Some(31));
    /// ```
    pub value: Option<u32>,

    /// The constructor identifier of the request that triggered this error.
    /// Won't be present if the error was artificially constructed.
    pub caused_by: Option<u32>,
}

impl std::error::Error for RpcError {}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rpc error {}: {}", self.code, self.name)?;
        if let Some(caused_by) = self.caused_by {
            write!(f, " caused by {}", tl::name_for_id(caused_by))?;
        }
        if let Some(value) = self.value {
            write!(f, " (value: {value})")?;
        }
        Ok(())
    }
}

impl From<tl::types::RpcError> for RpcError {
    fn from(error: tl::types::RpcError) -> Self {
        // Extract the numeric value in the error, if any
        if let Some(value) = error
            .error_message
            .split(|c: char| !c.is_ascii_digit())
            .find(|s| !s.is_empty())
        {
            let mut to_remove = String::with_capacity(1 + value.len());
            to_remove.push('_');
            to_remove.push_str(value);
            Self {
                code: error.error_code,
                name: error.error_message.replace(&to_remove, ""),
                // Safe to unwrap, matched on digits
                value: Some(value.parse().unwrap()),
                caused_by: None,
            }
        } else {
            Self {
                code: error.error_code,
                name: error.error_message.clone(),
                value: None,
                caused_by: None,
            }
        }
    }
}

impl RpcError {
    /// Matches on the name of the RPC error (case-sensitive).
    ///
    /// Useful in `match` arm guards. A single trailing or leading asterisk (`'*'`) is allowed,
    /// and will instead check if the error name starts (or ends with) the input parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// # let request_result = Result::<(), _>::Err(grammers_mtsender::RpcError {
    /// #     code: 400, name: "PHONE_CODE_INVALID".to_string(), value: None, caused_by: None });
    /// #
    /// match request_result {
    ///     Err(rpc_err) if rpc_err.is("SESSION_PASSWORD_NEEDED") => panic!(),
    ///     Err(rpc_err) if rpc_err.is("PHONE_CODE_*") => {},
    ///     _ => panic!()
    /// }
    /// ```
    pub fn is(&self, rpc_error: &str) -> bool {
        if let Some(rpc_error) = rpc_error.strip_suffix('*') {
            self.name.starts_with(rpc_error)
        } else if let Some(rpc_error) = rpc_error.strip_prefix('*') {
            self.name.ends_with(rpc_error)
        } else {
            self.name == rpc_error
        }
    }

    /// Attaches the [`tl::Identifiable::CONSTRUCTOR_ID`] of the
    /// request that caused this error to the error information.
    pub fn with_caused_by(mut self, constructor_id: u32) -> Self {
        self.caused_by = Some(constructor_id);
        self
    }
}

/// This error occurs when a Remote Procedure call was unsuccessful.
#[derive(Debug)]
pub enum InvocationError {
    /// The request invocation failed because it was invalid or the server
    /// could not process it successfully. If the server is suffering from
    /// temporary issues, the request may be retried after some time.
    Rpc(RpcError),

    /// Standard I/O error when reading the response.
    ///
    /// Telegram may kill the connection at any moment, but it is generally valid to retry
    /// the request at least once immediately, which will be done through a new connection.
    Io(io::Error),

    /// Error propagated from attempting to deserialize an invalid [`tl::Deserializable`].
    ///
    /// This occurs somewhat frequently when misusing a single session more than once at a time.
    /// Otherwise it might happen on bleeding-edge layers that have not had time to settle yet.
    Deserialize(mtp::DeserializeError),

    /// Error propagated from the underlying [`transport`].
    ///
    /// The most common variant is [`transport::Error::BadStatus`], which can occur when
    /// there's no valid Authorization Key (404) or too many connections have been made (429).
    Transport(transport::Error),

    /// The request was cancelled or dropped, and the results won't arrive.
    /// This may mean that the [`crate::SenderPoolRunner`] is no longer running.
    Dropped,

    /// The request was invoked in a datacenter that does not exist or is not known by the session.
    InvalidDc,

    /// The request caused the sender to connect to a new datacenter to be performed,
    /// but the Authorization Key generation process failed.
    Authentication(authentication::Error),
}

impl std::error::Error for InvocationError {}

impl fmt::Display for InvocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rpc(err) => write!(f, "request error: {err}"),
            Self::Io(err) => write!(f, "request error: {err}"),
            Self::Deserialize(err) => write!(f, "request error: {err}"),
            Self::Transport(err) => write!(f, "request error: {err}"),
            Self::Dropped => write!(f, "request error: dropped (cancelled)"),
            Self::InvalidDc => write!(f, "request error: invalid dc"),
            Self::Authentication(err) => write!(f, "request error: {err}"),
        }
    }
}

impl From<ReadError> for InvocationError {
    fn from(error: ReadError) -> Self {
        match error {
            ReadError::Io(error) => Self::from(error),
            ReadError::Transport(error) => Self::from(error),
            ReadError::Deserialize(error) => Self::from(error),
        }
    }
}

// PATCH (omniget): upstream 0.8.x has mutually recursive `From` impls here.
// `From<ReadError>` delegated each variant back to `From<io::Error>` /
// `From<transport::Error>` / `From<mtp::DeserializeError>`, and each of those
// wrapped the value back into a `ReadError` and called `From<ReadError>` again,
// so converting any read error recursed until the stack overflowed
// (STATUS_STACK_OVERFLOW on Windows, `InvocationError::from` repeated ~174k
// times in the macOS crash report). These now build the variant directly,
// which is what upstream 0.9.0 does. See omniget#289.
impl From<mtp::DeserializeError> for InvocationError {
    fn from(error: mtp::DeserializeError) -> Self {
        Self::Deserialize(error)
    }
}

impl From<transport::Error> for InvocationError {
    fn from(error: transport::Error) -> Self {
        Self::Transport(error)
    }
}

impl From<tl::deserialize::Error> for InvocationError {
    fn from(error: tl::deserialize::Error) -> Self {
        Self::Deserialize(error.into())
    }
}

impl From<io::Error> for InvocationError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<authentication::Error> for InvocationError {
    fn from(error: authentication::Error) -> Self {
        Self::Authentication(error)
    }
}

impl InvocationError {
    /// Matches on the name of the RPC error (case-sensitive).
    ///
    /// Useful in `match` arm guards. A single trailing or leading asterisk (`'*'`) is allowed,
    /// and will instead check if the error name starts (or ends with) the input parameter.
    ///
    /// If the error is not a RPC error, returns `false`.
    ///
    /// # Examples
    ///
    /// ```
    /// # let request_result = Result::<(), _>::Err(grammers_mtsender::InvocationError::Rpc(
    /// #     grammers_mtsender::RpcError { code: 400, name: "PHONE_CODE_INVALID".to_string(), value: None, caused_by: None }));
    /// #
    /// match request_result {
    ///     Err(err) if err.is("SESSION_PASSWORD_NEEDED") => panic!(),
    ///     Err(err) if err.is("PHONE_CODE_*") => {},
    ///     _ => panic!()
    /// }
    /// ```
    #[inline]
    pub fn is(&self, rpc_error: &str) -> bool {
        match self {
            Self::Rpc(rpc) => rpc.is(rpc_error),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_rpc_error_parsing() {
        assert_eq!(
            RpcError::from(tl::types::RpcError {
                error_code: 400,
                error_message: "CHAT_INVALID".into(),
            }),
            RpcError {
                code: 400,
                name: "CHAT_INVALID".into(),
                value: None,
                caused_by: None,
            }
        );
    }

    /// Regression test for omniget#289: on the unpatched 0.8.x sources every one
    /// of these conversions recursed forever and overflowed the stack instead of
    /// returning. Reaching the asserts at all is the point of the test.
    #[test]
    fn read_error_converts_without_recursing() {
        let io = ReadError::Io(io::Error::new(io::ErrorKind::ConnectionReset, "reset"));
        assert!(matches!(InvocationError::from(io), InvocationError::Io(_)));

        let transport = ReadError::Transport(transport::Error::BadStatus { status: 404 });
        assert!(matches!(
            InvocationError::from(transport),
            InvocationError::Transport(_)
        ));

        let deserialize = ReadError::Deserialize(mtp::DeserializeError::MessageBufferTooSmall);
        assert!(matches!(
            InvocationError::from(deserialize),
            InvocationError::Deserialize(_)
        ));

        // The leaf impls must not bounce back through `ReadError` either.
        assert!(matches!(
            InvocationError::from(io::Error::new(io::ErrorKind::BrokenPipe, "pipe")),
            InvocationError::Io(_)
        ));
        assert!(matches!(
            InvocationError::from(transport::Error::BadStatus { status: 429 }),
            InvocationError::Transport(_)
        ));
        assert!(matches!(
            InvocationError::from(mtp::DeserializeError::MessageBufferTooSmall),
            InvocationError::Deserialize(_)
        ));
    }
}
