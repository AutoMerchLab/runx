mod signals;

/// Default retained bytes per stdout/stderr stream for general operator
/// processes. The supervisor continues draining and hashing the complete stream;
/// capability-specific contracts may choose a narrower or wider retained body.
#[cfg(any(
    feature = "cli-tool",
    feature = "external-adapter",
    feature = "thread-outbox-provider"
))]
pub(crate) const STANDARD_PROCESS_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

#[cfg(any(
    feature = "cli-tool",
    feature = "external-adapter",
    feature = "thread-outbox-provider"
))]
mod capture;
#[cfg(any(
    feature = "cli-tool",
    feature = "external-adapter",
    feature = "thread-outbox-provider"
))]
mod resource_limits;
#[cfg(any(
    feature = "cli-tool",
    feature = "external-adapter",
    feature = "thread-outbox-provider"
))]
mod spec;
#[cfg(any(
    feature = "cli-tool",
    feature = "external-adapter",
    feature = "thread-outbox-provider"
))]
mod supervisor;
#[cfg(any(
    feature = "cli-tool",
    feature = "external-adapter",
    feature = "thread-outbox-provider"
))]
mod timeout;
#[cfg(feature = "mcp")]
mod tokio_supervisor;

#[cfg(feature = "cli-tool")]
pub(crate) use self::capture::CapturedOutput;
pub(crate) use self::signals::{ProcessSignal, configure_process_group, signal_process_group_id};
#[cfg(any(
    feature = "cli-tool",
    feature = "external-adapter",
    feature = "thread-outbox-provider"
))]
pub(crate) use self::spec::{ProcessOutcome, ProcessSpec, ProcessStdin, ProcessSupervisorError};
#[cfg(any(
    feature = "cli-tool",
    feature = "external-adapter",
    feature = "thread-outbox-provider"
))]
pub(crate) use self::supervisor::run_process;
#[cfg(any(
    feature = "cli-tool",
    feature = "external-adapter",
    feature = "thread-outbox-provider"
))]
use self::supervisor::{kill_timed_out_context, poll_timed_out_context};
#[cfg(feature = "mcp")]
pub(crate) use self::tokio_supervisor::{TokioProcessSpec, spawn_tokio_process};
