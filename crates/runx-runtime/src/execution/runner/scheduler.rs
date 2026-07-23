use std::collections::BTreeMap;

use runx_parser::GraphStep;

use super::RUNX_MAX_FANOUT_CONCURRENCY_ENV;
use crate::effects::RuntimeEffectRegistry;

const DEFAULT_MAX_FANOUT_CONCURRENCY: usize = 1;
const HARD_MAX_FANOUT_CONCURRENCY: usize = 64;

pub(super) struct FanoutScheduler {
    max_concurrency: usize,
}

pub(super) enum FanoutSchedule<'a> {
    Serial(Vec<ScheduledFanoutStep<'a>>),
    Parallel(ParallelFanoutSchedule<'a>),
}

pub(super) struct ParallelFanoutSchedule<'a> {
    pub(super) steps: Vec<ScheduledFanoutStep<'a>>,
    pub(super) max_concurrency: usize,
}

#[derive(Clone, Copy)]
pub(super) struct ScheduledFanoutStep<'a> {
    pub(super) step_id: &'a str,
    pub(super) attempt: u32,
    pub(super) parallel_limit: Option<usize>,
}

impl FanoutScheduler {
    pub(super) fn from_env(env: &BTreeMap<String, String>) -> Self {
        Self {
            max_concurrency: configured_max_concurrency(env),
        }
    }

    pub(super) fn schedule<'a>(&self, steps: Vec<ScheduledFanoutStep<'a>>) -> FanoutSchedule<'a> {
        if self.max_concurrency <= 1 || steps.len() <= 1 {
            return FanoutSchedule::Serial(steps);
        }
        let Some(step_limit) = steps.iter().try_fold(usize::MAX, |limit, step| {
            step.parallel_limit.map(|step_limit| limit.min(step_limit))
        }) else {
            return FanoutSchedule::Serial(steps);
        };
        FanoutSchedule::Parallel(ParallelFanoutSchedule {
            steps,
            max_concurrency: self.max_concurrency.min(step_limit),
        })
    }
}

pub(super) fn scheduled_step<'a>(
    step_id: &'a str,
    attempt: u32,
    parallel_limit: Option<usize>,
) -> ScheduledFanoutStep<'a> {
    ScheduledFanoutStep {
        step_id,
        attempt,
        parallel_limit,
    }
}

pub(super) fn parallel_safe_step_shape(step: &GraphStep, effects: &RuntimeEffectRegistry) -> bool {
    step.run.is_none()
        && step.tool.is_none()
        && !step.mutating
        && effects.allows_parallel_step(step)
}

pub(super) fn configured_max_concurrency(env: &BTreeMap<String, String>) -> usize {
    env.get(RUNX_MAX_FANOUT_CONCURRENCY_ENV)
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_FANOUT_CONCURRENCY)
        .min(HARD_MAX_FANOUT_CONCURRENCY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_serial_fanout() {
        assert_eq!(
            configured_max_concurrency(&BTreeMap::new()),
            DEFAULT_MAX_FANOUT_CONCURRENCY
        );
    }

    #[test]
    fn clamps_configured_fanout_concurrency() {
        let mut env = BTreeMap::new();
        env.insert(
            RUNX_MAX_FANOUT_CONCURRENCY_ENV.to_owned(),
            "100000".to_owned(),
        );
        assert_eq!(
            configured_max_concurrency(&env),
            HARD_MAX_FANOUT_CONCURRENCY
        );
    }

    #[test]
    fn keeps_mixed_capability_fanout_serial() {
        let scheduler = FanoutScheduler {
            max_concurrency: HARD_MAX_FANOUT_CONCURRENCY,
        };
        let steps = vec![
            ScheduledFanoutStep {
                step_id: "a",
                attempt: 1,
                parallel_limit: Some(HARD_MAX_FANOUT_CONCURRENCY),
            },
            ScheduledFanoutStep {
                step_id: "b",
                attempt: 1,
                parallel_limit: None,
            },
        ];
        assert!(matches!(
            scheduler.schedule(steps),
            FanoutSchedule::Serial(_)
        ));
    }

    #[test]
    fn clamps_parallel_schedule_to_the_narrowest_adapter_limit() {
        let scheduler = FanoutScheduler {
            max_concurrency: HARD_MAX_FANOUT_CONCURRENCY,
        };
        let schedule = scheduler.schedule(vec![
            ScheduledFanoutStep {
                step_id: "a",
                attempt: 1,
                parallel_limit: Some(4),
            },
            ScheduledFanoutStep {
                step_id: "b",
                attempt: 1,
                parallel_limit: Some(8),
            },
        ]);
        assert!(matches!(
            schedule,
            FanoutSchedule::Parallel(ParallelFanoutSchedule {
                max_concurrency: 4,
                ..
            })
        ));
    }
}
