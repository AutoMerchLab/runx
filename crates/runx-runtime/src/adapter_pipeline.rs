#[cfg(any(
    feature = "a2a",
    feature = "agent",
    feature = "catalog",
    feature = "mcp"
))]
use std::time::Instant;

use runx_contracts::JsonObject;

use crate::adapter::{InvocationStatus, SkillOutput};

#[derive(Clone, Debug)]
#[cfg(feature = "mcp")]
pub(crate) struct AdapterExecutionContext {
    started: Instant,
}

#[cfg(feature = "mcp")]
impl AdapterExecutionContext {
    pub(crate) fn start() -> Self {
        Self {
            started: Instant::now(),
        }
    }

    pub(crate) fn duration_ms(&self) -> u64 {
        duration_ms(self.started)
    }

    pub(crate) fn projection(&self) -> AdapterProjection {
        AdapterProjection::from_duration_ms(self.duration_ms())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AdapterCapture {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl AdapterCapture {
    pub(crate) fn new(stdout: String, stderr: String) -> Self {
        Self { stdout, stderr }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AdapterProjection {
    duration_ms: u64,
}

impl AdapterProjection {
    pub(crate) const fn from_duration_ms(duration_ms: u64) -> Self {
        Self { duration_ms }
    }

    #[cfg(any(feature = "a2a", feature = "agent", feature = "catalog"))]
    pub(crate) fn from_started(started: Instant) -> Self {
        Self::from_duration_ms(duration_ms(started))
    }

    pub(crate) fn output(
        &self,
        status: InvocationStatus,
        capture: AdapterCapture,
        exit_code: Option<i32>,
        metadata: JsonObject,
    ) -> SkillOutput {
        SkillOutput {
            status,
            stdout: capture.stdout,
            stderr: capture.stderr,
            exit_code,
            duration_ms: self.duration_ms,
            metadata,
        }
    }

    #[cfg(any(
        feature = "a2a",
        feature = "agent",
        feature = "catalog",
        feature = "mcp"
    ))]
    pub(crate) fn failure(self, message: String, metadata: JsonObject) -> SkillOutput {
        self.output(
            InvocationStatus::Failure,
            AdapterCapture::new(String::new(), message),
            None,
            metadata,
        )
    }
}

#[cfg(any(
    feature = "a2a",
    feature = "agent",
    feature = "catalog",
    feature = "mcp"
))]
pub(crate) fn duration_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
