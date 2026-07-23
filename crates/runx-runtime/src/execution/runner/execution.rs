// Module rationale: graph execution keeps step planning,
// fanout synchronization, and checkpoint emission together while Rust remains
// the parity implementation for the existing execution contract.
use std::collections::BTreeMap;
use std::path::Path;
use std::thread;

use runx_contracts::{ExecutionEvent, FanoutReceiptSyncPoint, JsonValue};
use runx_core::state_machine::{
    FanoutBranchPlan, FanoutGroupPolicy, FanoutSyncDecision, FanoutSyncOutcome, GraphStepStatus,
    SequentialGraphEvent, SequentialGraphPlan, SequentialGraphState, create_sequential_graph_state,
};
use runx_parser::{ExecutionGraph, GraphStep};

use super::super::fanout::fanout_policies;
use super::super::graph::{LoadedStepSkill, StepSkillCache, StepSkillLoadOptions};
use super::super::graph_index::{ExecutionGraphIndex, PriorRunIndex};
use super::scheduler::{
    FanoutSchedule, FanoutScheduler, ParallelFanoutSchedule, ScheduledFanoutStep,
    parallel_safe_step_shape, scheduled_step,
};
use super::step_execution::{
    LoadedStepExecutionRequest, run_step_with_loaded_skill, run_step_with_loaded_skill_index,
};
use super::steps::{output_error, runtime_error_step_run};
use super::sync::fanout_sync_point;
use super::{GraphCheckpoint, GraphRun, Runtime, RuntimeOptions, StepRun};
use crate::RuntimeError;
use crate::adapter::SkillAdapter;
use crate::host::{Host, NoopHost};
use crate::journal::ExecutionJournal;
use crate::lifecycle::LifecycleEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StepFailureMode {
    Propagate,
    RecordAndContinue,
}

struct FanoutRunPlan {
    group_id: String,
    branches: Vec<FanoutBranchPlan>,
}

pub(super) struct GraphExecution {
    graph_index: ExecutionGraphIndex,
    planning_cursor: usize,
    step_skill_cache: StepSkillCache,
    state: SequentialGraphState,
    pub(super) runs: Vec<StepRun>,
    run_positions: BTreeMap<String, usize>,
    pub(super) sync_points: Vec<FanoutReceiptSyncPoint>,
    journal: ExecutionJournal,
}

struct ParallelStepRun {
    sequence: usize,
    step_id: String,
    attempt: u32,
    run: StepRun,
}

struct ParallelFanoutJob<'a> {
    sequence: usize,
    step_id: String,
    attempt: u32,
    step: &'a GraphStep,
    loaded_skill: Option<LoadedStepSkill>,
    uses_javascript: bool,
}

#[derive(Clone, Copy)]
pub(super) struct StepExecutionPlan<'a> {
    step_id: &'a str,
    attempt: u32,
    failure_mode: StepFailureMode,
}

const DISABLE_RUNTIME_INDEXES_ENV: &str = "RUNX_RUNTIME_DISABLE_INDEXES";

impl GraphExecution {
    pub(super) fn new(graph: &ExecutionGraph) -> Self {
        let definitions = super::super::graph::step_definitions(graph);
        let state = create_sequential_graph_state(graph.name.clone(), &definitions);
        let graph_index = ExecutionGraphIndex::new(graph, definitions);
        Self {
            graph_index,
            planning_cursor: 0,
            step_skill_cache: StepSkillCache::default(),
            state,
            runs: Vec::new(),
            run_positions: BTreeMap::new(),
            sync_points: Vec::new(),
            journal: ExecutionJournal::default(),
        }
    }

    fn apply_state_event(&mut self, event: SequentialGraphEvent) {
        self.graph_index.apply_event(&mut self.state, event);
    }

    pub(super) fn from_checkpoint(
        graph: &ExecutionGraph,
        checkpoint: GraphCheckpoint,
    ) -> Result<Self, RuntimeError> {
        if checkpoint.graph_name != graph.name {
            return Err(RuntimeError::CheckpointGraphMismatch {
                checkpoint_graph: checkpoint.graph_name,
                graph: graph.name.clone(),
            });
        }
        let definitions = super::super::graph::step_definitions(graph);
        let graph_index = ExecutionGraphIndex::new(graph, definitions);
        let planning_cursor =
            checkpoint_planning_cursor(graph, &checkpoint.state, &checkpoint.sync_points)?;
        let run_positions = run_positions(&checkpoint.steps);
        Ok(Self {
            graph_index,
            planning_cursor,
            step_skill_cache: StepSkillCache::default(),
            state: checkpoint.state,
            runs: checkpoint.steps,
            run_positions,
            sync_points: checkpoint.sync_points,
            journal: checkpoint.journal,
        })
    }

    pub(super) fn run<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        host: &mut dyn Host,
        max_new_steps: Option<usize>,
    ) -> Result<(), RuntimeError>
    where
        A: SkillAdapter,
    {
        let fanout_policies = fanout_policies(graph);
        let initial_step_count = self.runs.len();
        loop {
            if reached_step_limit(initial_step_count, self.runs.len(), max_new_steps) {
                return Ok(());
            }
            self.mark_when_skipped_steps(graph, &runtime.options.created_at);
            self.advance_planning_cursor(graph);
            let plan = self.graph_index.plan_transition(
                &self.state,
                &fanout_policies,
                self.planning_cursor,
            );
            if self.apply_plan(runtime, graph_dir, graph, host, &fanout_policies, plan)? {
                break;
            }
        }
        Ok(())
    }

    fn advance_planning_cursor(&mut self, graph: &ExecutionGraph) {
        self.planning_cursor =
            terminal_prefix_cursor(graph, &self.state, &self.sync_points, self.planning_cursor);
    }

    /// Mark every step whose `when` condition the runtime has resolved to false
    /// as `Skipped`, so the planner walks past it and graph completion treats it
    /// as terminal. Evaluated against the runs so far, so a branch is only
    /// selected out once the step it reads from has produced its output.
    fn mark_when_skipped_steps(&mut self, graph: &ExecutionGraph, at: &str) {
        let already_skipped = self
            .state
            .steps
            .iter()
            .filter(|step| step.status == GraphStepStatus::Skipped)
            .map(|step| step.step_id.clone())
            .collect();
        for step_id in when_skipped_steps(graph, &self.runs, &already_skipped) {
            let is_pending = self
                .state
                .steps
                .iter()
                .any(|step| step.step_id == step_id && step.status == GraphStepStatus::Pending);
            if is_pending {
                self.apply_state_event(SequentialGraphEvent::StepSkipped {
                    step_id,
                    at: at.to_owned(),
                });
            }
        }
    }

    pub(super) fn apply_plan<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        host: &mut dyn Host,
        fanout_policies: &BTreeMap<String, FanoutGroupPolicy>,
        plan: SequentialGraphPlan,
    ) -> Result<bool, RuntimeError>
    where
        A: SkillAdapter,
    {
        match plan {
            SequentialGraphPlan::RunStep {
                step_id, attempt, ..
            } => self.apply_step_plan(runtime, graph_dir, graph, host, &step_id, attempt),
            SequentialGraphPlan::RunFanout { group_id, branches } => {
                self.run_fanout_plan(
                    runtime,
                    graph_dir,
                    graph,
                    host,
                    fanout_policies,
                    FanoutRunPlan { group_id, branches },
                )?;
                Ok(false)
            }
            SequentialGraphPlan::Complete => Ok(self.complete_graph()),
            SequentialGraphPlan::Blocked {
                step_id,
                reason,
                sync_decision,
            } => self.block_graph(graph, step_id, reason, sync_decision),
            SequentialGraphPlan::Failed {
                step_id,
                reason,
                sync_decision,
            } => self.fail_graph(graph, step_id, reason, sync_decision),
            SequentialGraphPlan::Paused {
                step_id,
                reason,
                sync_decision,
            } => self.pause_for_sync(graph, step_id, reason, sync_decision),
            SequentialGraphPlan::Escalated {
                step_id,
                reason,
                sync_decision,
            } => self.escalate_for_sync(graph, step_id, reason, sync_decision),
        }
    }

    pub(super) fn apply_step_plan<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        host: &mut dyn Host,
        step_id: &str,
        attempt: u32,
    ) -> Result<bool, RuntimeError>
    where
        A: SkillAdapter,
    {
        self.run_one_step(runtime, graph_dir, graph, step_id, attempt, host)?;
        Ok(false)
    }

    pub(super) fn complete_graph(&mut self) -> bool {
        self.apply_state_event(SequentialGraphEvent::Complete);
        true
    }

    fn run_fanout_plan<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        host: &mut dyn Host,
        fanout_policies: &BTreeMap<String, FanoutGroupPolicy>,
        plan: FanoutRunPlan,
    ) -> Result<(), RuntimeError>
    where
        A: SkillAdapter,
    {
        if runtime
            .options
            .env
            .contains_key(DISABLE_RUNTIME_INDEXES_ENV)
        {
            self.run_serial_fanout_steps(runtime, graph_dir, graph, host, &plan.branches)?;
            return self.record_proceeding_fanout_sync_point(
                graph,
                fanout_policies,
                &plan.group_id,
            );
        }

        let scheduler = FanoutScheduler::from_env(&runtime.options.env);
        let steps = self.scheduled_fanout_steps(runtime, graph_dir, graph, &plan.branches)?;
        match scheduler.schedule(steps) {
            FanoutSchedule::Serial(steps) => {
                self.run_scheduled_fanout_steps(runtime, graph_dir, graph, host, steps)?;
            }
            FanoutSchedule::Parallel(schedule) => {
                self.run_parallel_fanout_steps(runtime, graph_dir, graph, host, schedule)?;
            }
        }
        self.record_proceeding_fanout_sync_point(graph, fanout_policies, &plan.group_id)
    }

    fn run_serial_fanout_steps<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        host: &mut dyn Host,
        branches: &[FanoutBranchPlan],
    ) -> Result<(), RuntimeError>
    where
        A: SkillAdapter,
    {
        let steps = branches
            .iter()
            .map(|branch| ScheduledFanoutStep {
                step_id: &branch.step_id,
                attempt: branch.attempt,
                parallel_limit: None,
            })
            .collect();
        self.run_scheduled_fanout_steps(runtime, graph_dir, graph, host, steps)
    }

    fn scheduled_fanout_steps<'a, A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        branches: &'a [FanoutBranchPlan],
    ) -> Result<Vec<ScheduledFanoutStep<'a>>, RuntimeError>
    where
        A: SkillAdapter,
    {
        branches
            .iter()
            .map(|branch| {
                let step = self.find_step(graph, &branch.step_id)?;
                Ok(scheduled_step(
                    &branch.step_id,
                    branch.attempt,
                    self.parallel_fanout_limit(runtime, graph_dir, step),
                ))
            })
            .collect()
    }

    fn parallel_fanout_limit<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        step: &GraphStep,
    ) -> Option<usize>
    where
        A: SkillAdapter,
    {
        if !parallel_safe_step_shape(step, &runtime.options().effects) {
            return None;
        }
        let Ok(Some(skill)) = self.cached_step_skill(runtime, graph_dir, step) else {
            return None;
        };
        if skill.runner.source.source_type == runx_parser::SourceKind::JavaScript {
            return Some(runtime.javascript.max_concurrency());
        }
        (runtime.adapter.fanout_execution_mode(&skill.runner.source)
            == crate::adapter::FanoutExecutionMode::IsolatedParallel)
            .then_some(usize::MAX)
    }

    fn run_scheduled_fanout_steps<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        host: &mut dyn Host,
        steps: Vec<ScheduledFanoutStep<'_>>,
    ) -> Result<(), RuntimeError>
    where
        A: SkillAdapter,
    {
        for step in steps {
            self.run_one_step_with_mode(
                runtime,
                graph_dir,
                graph,
                host,
                StepExecutionPlan {
                    step_id: step.step_id,
                    attempt: step.attempt,
                    failure_mode: StepFailureMode::RecordAndContinue,
                },
            )?;
        }
        Ok(())
    }

    fn run_parallel_fanout_steps<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        host: &mut dyn Host,
        schedule: ParallelFanoutSchedule<'_>,
    ) -> Result<(), RuntimeError>
    where
        A: SkillAdapter,
    {
        for scheduled in &schedule.steps {
            let step = self.find_step(graph, scheduled.step_id)?;
            enforce_guards(graph, step, &self.runs)?;
        }
        for scheduled in &schedule.steps {
            self.record_lifecycle(host, LifecycleEvent::step_started(scheduled.step_id))?;
            self.start_step(runtime, scheduled.step_id);
        }

        let results = self.execute_parallel_fanout_steps(
            runtime,
            graph_dir,
            graph,
            &schedule.steps,
            schedule.max_concurrency,
        )?;
        for result in results {
            self.commit_step_run(
                runtime,
                host,
                StepExecutionPlan {
                    step_id: &result.step_id,
                    attempt: result.attempt,
                    failure_mode: StepFailureMode::RecordAndContinue,
                },
                result.run,
                false,
            )?;
        }
        Ok(())
    }

    fn execute_parallel_fanout_steps<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        steps: &[ScheduledFanoutStep<'_>],
        max_concurrency: usize,
    ) -> Result<Vec<ParallelStepRun>, RuntimeError>
    where
        A: SkillAdapter,
    {
        let mut results = Vec::with_capacity(steps.len());
        let chunk_size = max_concurrency.max(1);
        for (chunk_index, chunk) in steps.chunks(chunk_size).enumerate() {
            let mut chunk_results = self.execute_parallel_fanout_batch(
                runtime,
                graph_dir,
                graph,
                chunk,
                chunk_index * chunk_size,
            )?;
            results.append(&mut chunk_results);
        }
        results.sort_by_key(|result| result.sequence);
        Ok(results)
    }

    fn execute_parallel_fanout_batch<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        steps: &[ScheduledFanoutStep<'_>],
        sequence_base: usize,
    ) -> Result<Vec<ParallelStepRun>, RuntimeError>
    where
        A: SkillAdapter,
    {
        let jobs = self.parallel_fanout_jobs(runtime, graph_dir, graph, steps, sequence_base)?;
        let runs = &self.runs;
        let run_positions = &self.run_positions;
        thread::scope(|scope| {
            let mut handles = Vec::with_capacity(jobs.len());
            for job in jobs {
                let adapter: Box<dyn SkillAdapter + Send + Sync> = if job.uses_javascript {
                    Box::new(runtime.javascript.clone())
                } else {
                    runtime.adapter.clone_for_fanout().ok_or_else(|| {
                        RuntimeError::UnsupportedAdapter {
                            adapter_type: format!(
                                "{} parallel fanout",
                                runtime.adapter.adapter_type()
                            ),
                        }
                    })?
                };
                let options = runtime.options.clone();
                let javascript = runtime.javascript.clone();
                let local_artifacts = runtime.local_artifacts.clone();
                let graph_name = graph.name.as_str();
                handles.push(scope.spawn(move || {
                    let run = execute_parallel_fanout_step(ParallelFanoutStepExecution {
                        adapter,
                        javascript,
                        local_artifacts,
                        options,
                        graph_dir,
                        graph_name,
                        step: job.step,
                        attempt: job.attempt,
                        loaded_skill: job.loaded_skill,
                        prior_runs: runs,
                        run_positions,
                    })?;
                    Ok::<ParallelStepRun, RuntimeError>(ParallelStepRun {
                        sequence: job.sequence,
                        step_id: job.step_id,
                        attempt: job.attempt,
                        run,
                    })
                }));
            }
            join_parallel_fanout_handles(handles)
        })
    }

    fn parallel_fanout_jobs<'a>(
        &mut self,
        runtime: &Runtime<impl SkillAdapter>,
        graph_dir: &Path,
        graph: &'a ExecutionGraph,
        steps: &[ScheduledFanoutStep<'_>],
        sequence_base: usize,
    ) -> Result<Vec<ParallelFanoutJob<'a>>, RuntimeError> {
        steps
            .iter()
            .enumerate()
            .map(|(offset, scheduled)| {
                let step = self.find_step(graph, scheduled.step_id)?;
                let loaded_skill = self.cached_step_skill(runtime, graph_dir, step)?;
                let uses_javascript = loaded_skill.as_ref().is_some_and(|skill| {
                    skill.runner.source.source_type == runx_parser::SourceKind::JavaScript
                });
                Ok(ParallelFanoutJob {
                    sequence: sequence_base + offset,
                    step_id: scheduled.step_id.to_owned(),
                    attempt: scheduled.attempt,
                    step,
                    loaded_skill,
                    uses_javascript,
                })
            })
            .collect()
    }

    pub(super) fn block_graph(
        &mut self,
        graph: &ExecutionGraph,
        step_id: String,
        reason: String,
        sync_decision: Option<FanoutSyncDecision>,
    ) -> Result<bool, RuntimeError> {
        if let Some(sync_decision) = sync_decision {
            self.push_sync_point(graph, &sync_decision)?;
        }
        Err(RuntimeError::GraphBlocked { step_id, reason })
    }

    pub(super) fn fail_graph(
        &mut self,
        graph: &ExecutionGraph,
        step_id: String,
        reason: String,
        sync_decision: Option<FanoutSyncDecision>,
    ) -> Result<bool, RuntimeError> {
        if let Some(sync_decision) = sync_decision {
            self.push_sync_point(graph, &sync_decision)?;
        }
        self.apply_state_event(SequentialGraphEvent::FailGraph {
            error: reason.clone(),
        });
        Err(RuntimeError::GraphPlanningFailed { step_id, reason })
    }

    pub(super) fn pause_graph(
        &mut self,
        step_id: String,
        reason: String,
        sync_decision: runx_core::state_machine::FanoutSyncDecision,
    ) -> Result<bool, RuntimeError> {
        self.apply_state_event(SequentialGraphEvent::PauseGraph {
            reason: reason.clone(),
        });
        Err(RuntimeError::GraphPaused {
            step_id,
            reason,
            sync_decision: Box::new(sync_decision),
        })
    }

    pub(super) fn pause_for_sync(
        &mut self,
        graph: &ExecutionGraph,
        step_id: String,
        reason: String,
        sync_decision: FanoutSyncDecision,
    ) -> Result<bool, RuntimeError> {
        self.push_sync_point(graph, &sync_decision)?;
        self.pause_graph(step_id, reason, sync_decision)
    }

    pub(super) fn escalate_graph(
        &mut self,
        step_id: String,
        reason: String,
        sync_decision: runx_core::state_machine::FanoutSyncDecision,
    ) -> Result<bool, RuntimeError> {
        self.apply_state_event(SequentialGraphEvent::EscalateGraph {
            reason: reason.clone(),
        });
        Err(RuntimeError::GraphEscalated {
            step_id,
            reason,
            sync_decision: Box::new(sync_decision),
        })
    }

    pub(super) fn escalate_for_sync(
        &mut self,
        graph: &ExecutionGraph,
        step_id: String,
        reason: String,
        sync_decision: FanoutSyncDecision,
    ) -> Result<bool, RuntimeError> {
        self.push_sync_point(graph, &sync_decision)?;
        self.escalate_graph(step_id, reason, sync_decision)
    }

    pub(super) fn run_one_step<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        step_id: &str,
        attempt: u32,
        host: &mut dyn Host,
    ) -> Result<(), RuntimeError>
    where
        A: SkillAdapter,
    {
        self.run_one_step_with_mode(
            runtime,
            graph_dir,
            graph,
            host,
            StepExecutionPlan {
                step_id,
                attempt,
                failure_mode: StepFailureMode::Propagate,
            },
        )
    }

    pub(super) fn run_one_step_with_mode<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        host: &mut dyn Host,
        plan: StepExecutionPlan<'_>,
    ) -> Result<(), RuntimeError>
    where
        A: SkillAdapter,
    {
        let step = self.find_step(graph, plan.step_id)?;
        enforce_guards(graph, step, &self.runs)?;
        let retry_remaining = retry_budget_remaining(step, plan.attempt);
        self.record_lifecycle(host, LifecycleEvent::step_started(plan.step_id))?;
        self.start_step(runtime, plan.step_id);
        let run = self.execute_step_plan(runtime, graph_dir, graph, step, host, plan)?;
        self.commit_step_run(runtime, host, plan, run, retry_remaining)
    }

    fn execute_step_plan<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        step: &GraphStep,
        host: &mut dyn Host,
        plan: StepExecutionPlan<'_>,
    ) -> Result<StepRun, RuntimeError>
    where
        A: SkillAdapter,
    {
        let run_result = if runtime
            .options
            .env
            .contains_key(DISABLE_RUNTIME_INDEXES_ENV)
        {
            self.execute_step_without_index(runtime, graph_dir, graph, step, host, plan)
        } else {
            self.execute_step_with_index(runtime, graph_dir, graph, step, host, plan)
        };
        let run_result = run_result.map_err(|error| error.at_graph_step(&step.id));
        Ok(match run_result {
            Ok(run) => run,
            Err(error) if plan.failure_mode == StepFailureMode::RecordAndContinue => {
                runtime_error_step_run(runtime, &graph.name, step, plan.attempt, error)?
            }
            Err(error) => return Err(error),
        })
    }

    fn execute_step_without_index<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        step: &GraphStep,
        host: &mut dyn Host,
        plan: StepExecutionPlan<'_>,
    ) -> Result<StepRun, RuntimeError>
    where
        A: SkillAdapter,
    {
        let loaded_skill = self.cached_step_skill(runtime, graph_dir, step)?;
        run_step_with_loaded_skill(
            LoadedStepExecutionRequest {
                runtime,
                graph_dir,
                graph_name: &graph.name,
                step,
                attempt: plan.attempt,
                loaded_skill,
                host,
            },
            &self.runs,
        )
    }

    fn execute_step_with_index<A>(
        &mut self,
        runtime: &Runtime<A>,
        graph_dir: &Path,
        graph: &ExecutionGraph,
        step: &GraphStep,
        host: &mut dyn Host,
        plan: StepExecutionPlan<'_>,
    ) -> Result<StepRun, RuntimeError>
    where
        A: SkillAdapter,
    {
        let loaded_skill = self.cached_step_skill(runtime, graph_dir, step)?;
        let prior_run_index = PriorRunIndex::from_positions(&self.runs, &self.run_positions);
        run_step_with_loaded_skill_index(
            LoadedStepExecutionRequest {
                runtime,
                graph_dir,
                graph_name: &graph.name,
                step,
                attempt: plan.attempt,
                loaded_skill,
                host,
            },
            &prior_run_index,
        )
    }

    fn commit_step_run<A>(
        &mut self,
        runtime: &Runtime<A>,
        host: &mut dyn Host,
        plan: StepExecutionPlan<'_>,
        run: StepRun,
        retry_remaining: bool,
    ) -> Result<(), RuntimeError>
    where
        A: SkillAdapter,
    {
        if run.output.succeeded() {
            self.succeed_step(runtime, &run);
            self.push_run(run);
            self.record_lifecycle(host, LifecycleEvent::step_completed(plan.step_id))
        } else {
            self.fail_step(runtime, plan.step_id, &run);
            host.log(format!("step {} failed", plan.step_id))?;
            self.record_lifecycle(host, LifecycleEvent::step_failed(plan.step_id))?;
            let terminal =
                plan.failure_mode != StepFailureMode::RecordAndContinue && !retry_remaining;
            // runx must never swallow a step error. A cli-tool failure reports
            // detail on stderr, but the governed HTTP front captures a non-2xx
            // response body on stdout with the status in metadata. Prefer stderr,
            // then fall back to the status and body so the message is never empty.
            let message = if run.output.stderr.trim().is_empty() {
                let status = run
                    .output
                    .metadata
                    .get("http_status")
                    .and_then(|value| value.as_str())
                    .map(|status| format!("status {status}: "))
                    .unwrap_or_default();
                let body = run.output.stdout.trim();
                if body.is_empty() {
                    format!("{status}step failed with no error output")
                } else {
                    format!("{status}{body}")
                }
            } else {
                run.output.stderr.clone()
            };
            // The failed run is recorded even on terminal failure so the run
            // list agrees with the journal's StepFailed event; a failed attempt
            // must never be silently absent from the execution record.
            self.push_run(run);
            if terminal {
                Err(RuntimeError::SkillFailed {
                    skill_name: plan.step_id.to_owned(),
                    message,
                })
            } else {
                Ok(())
            }
        }
    }

    fn push_run(&mut self, run: StepRun) {
        let index = self.runs.len();
        self.run_positions.insert(run.step_id.clone(), index);
        self.runs.push(run);
    }

    pub(super) fn start_step<A>(&mut self, runtime: &Runtime<A>, step_id: &str) {
        self.graph_index
            .start_step(&mut self.state, step_id, runtime.options.created_at.clone());
    }

    pub(super) fn succeed_step<A>(&mut self, runtime: &Runtime<A>, run: &StepRun) {
        self.graph_index.succeed_step(
            &mut self.state,
            runtime.options.created_at.clone(),
            run.admission_witness.clone(),
            Some(run.outputs.clone()),
        );
    }

    pub(super) fn fail_step<A>(&mut self, runtime: &Runtime<A>, step_id: &str, run: &StepRun) {
        self.apply_state_event(SequentialGraphEvent::StepFailed {
            step_id: step_id.to_owned(),
            at: runtime.options.created_at.clone(),
            error: output_error(run),
        });
    }

    pub(super) fn record_terminal_step_failure<A>(
        &mut self,
        runtime: &Runtime<A>,
        host: &mut dyn Host,
        step_id: &str,
        run: StepRun,
    ) -> Result<(), RuntimeError>
    where
        A: SkillAdapter,
    {
        self.record_lifecycle(host, LifecycleEvent::step_started(step_id))?;
        self.start_step(runtime, step_id);
        self.fail_step(runtime, step_id, &run);
        self.push_run(run);
        self.apply_state_event(SequentialGraphEvent::FailGraph {
            error: format!("step {step_id} failed"),
        });
        self.record_lifecycle(host, LifecycleEvent::step_failed(step_id))
    }

    pub(super) fn record(
        &mut self,
        host: &mut dyn Host,
        event: ExecutionEvent,
    ) -> Result<(), RuntimeError> {
        self.journal.push(event.clone());
        host.report(event)
    }

    pub(super) fn record_lifecycle(
        &mut self,
        host: &mut dyn Host,
        event: LifecycleEvent,
    ) -> Result<(), RuntimeError> {
        self.record(host, event.into_execution_event())
    }

    pub(super) fn finish(
        self,
        graph: ExecutionGraph,
        receipt: runx_contracts::Receipt,
    ) -> GraphRun {
        GraphRun {
            graph,
            state: self.state,
            steps: self.runs,
            sync_points: self.sync_points,
            receipt,
            journal: self.journal,
        }
    }

    pub(super) fn checkpoint(self, graph_name: String) -> GraphCheckpoint {
        GraphCheckpoint {
            graph_name,
            state: self.state,
            steps: self.runs,
            sync_points: self.sync_points,
            journal: self.journal,
        }
    }

    pub(super) fn record_proceeding_fanout_sync_point(
        &mut self,
        graph: &ExecutionGraph,
        fanout_policies: &BTreeMap<String, FanoutGroupPolicy>,
        group_id: &str,
    ) -> Result<(), RuntimeError> {
        let follow_up =
            self.graph_index
                .plan_transition(&self.state, fanout_policies, self.planning_cursor);
        if matches!(
            follow_up,
            SequentialGraphPlan::RunFanout {
                group_id: ref next_group_id,
                ..
            } if next_group_id == group_id
        ) {
            return Ok(());
        }

        let Some(policy) = fanout_policies.get(group_id) else {
            return Ok(());
        };
        let decision = self.graph_index.fanout_decision(&self.state, policy);
        if decision.decision == FanoutSyncOutcome::Proceed {
            self.push_sync_point(graph, &decision)?;
        }
        Ok(())
    }

    pub(super) fn push_sync_point(
        &mut self,
        graph: &ExecutionGraph,
        decision: &FanoutSyncDecision,
    ) -> Result<(), RuntimeError> {
        let sync_point = fanout_sync_point(
            decision,
            &self.graph_index.fanout_receipt_ids(
                graph,
                &self.runs,
                &self.run_positions,
                &decision.group_id,
            ),
        );
        let already_recorded = self.sync_points.iter().any(|existing| {
            existing.group_id == sync_point.group_id
                && existing.rule_fired == sync_point.rule_fired
                && existing.decision == sync_point.decision
        });
        if !already_recorded {
            self.sync_points.push(sync_point);
        }
        Ok(())
    }

    fn cached_step_skill(
        &mut self,
        runtime: &Runtime<impl SkillAdapter>,
        graph_dir: &Path,
        step: &GraphStep,
    ) -> Result<Option<LoadedStepSkill>, RuntimeError> {
        if step.run.is_some() || step.tool.is_some() {
            return Ok(None);
        }
        self.step_skill_cache
            .load(
                graph_dir,
                step,
                StepSkillLoadOptions {
                    env: &runtime.options().env,
                },
            )
            .map(Some)
    }

    fn find_step<'a>(
        &self,
        graph: &'a ExecutionGraph,
        step_id: &str,
    ) -> Result<&'a GraphStep, RuntimeError> {
        // `graph_index` is built from exactly this `graph` (see `GraphExecution::new`
        // / `from_checkpoint`), which is immutable for the run, so the index position
        // map is always in sync with `graph.steps`. The index's `StepMissing` is the
        // authoritative answer for a genuinely-missing step; a linear re-scan over the
        // same `graph.steps` could never find a step the index legitimately missed, it
        // would only silently paper over an index/graph desync. Return the index result
        // directly so such a desync surfaces instead of being absorbed by an O(n) scan.
        self.graph_index.find_step(graph, step_id)
    }
}

struct ParallelFanoutStepExecution<'a> {
    adapter: Box<dyn SkillAdapter + Send + Sync>,
    javascript: crate::adapters::javascript::JavaScriptAdapter,
    local_artifacts: crate::services::LocalArtifactService,
    options: RuntimeOptions,
    graph_dir: &'a Path,
    graph_name: &'a str,
    step: &'a GraphStep,
    attempt: u32,
    loaded_skill: Option<LoadedStepSkill>,
    prior_runs: &'a [StepRun],
    run_positions: &'a BTreeMap<String, usize>,
}

fn execute_parallel_fanout_step(
    execution: ParallelFanoutStepExecution<'_>,
) -> Result<StepRun, RuntimeError> {
    let ParallelFanoutStepExecution {
        adapter,
        javascript,
        local_artifacts,
        options,
        graph_dir,
        graph_name,
        step,
        attempt,
        loaded_skill,
        prior_runs,
        run_positions,
    } = execution;
    let runtime = Runtime::with_javascript(adapter, options, javascript, local_artifacts);
    let prior_run_index = PriorRunIndex::from_positions(prior_runs, run_positions);
    let mut host = NoopHost;
    match run_step_with_loaded_skill_index(
        LoadedStepExecutionRequest {
            runtime: &runtime,
            graph_dir,
            graph_name,
            step,
            attempt,
            loaded_skill,
            host: &mut host,
        },
        &prior_run_index,
    ) {
        Ok(run) => Ok(run),
        Err(error) => runtime_error_step_run(&runtime, graph_name, step, attempt, error),
    }
}

fn join_parallel_fanout_handles(
    handles: Vec<thread::ScopedJoinHandle<'_, Result<ParallelStepRun, RuntimeError>>>,
) -> Result<Vec<ParallelStepRun>, RuntimeError> {
    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        results.push(handle.join().map_err(|_| RuntimeError::SkillFailed {
            skill_name: "fanout".to_owned(),
            message: "parallel fanout worker panicked".to_owned(),
        })??);
    }
    Ok(results)
}

fn checkpoint_planning_cursor(
    graph: &ExecutionGraph,
    state: &SequentialGraphState,
    sync_points: &[FanoutReceiptSyncPoint],
) -> Result<usize, RuntimeError> {
    if let Some(step) = state
        .steps
        .iter()
        .find(|step| step.status == GraphStepStatus::Running)
    {
        return Err(RuntimeError::GraphPlanningFailed {
            step_id: step.step_id.clone(),
            reason: "checkpoint contains a running step".to_owned(),
        });
    }
    Ok(terminal_prefix_cursor(graph, state, sync_points, 0))
}

fn terminal_prefix_cursor(
    graph: &ExecutionGraph,
    state: &SequentialGraphState,
    sync_points: &[FanoutReceiptSyncPoint],
    start: usize,
) -> usize {
    let mut cursor = start.min(state.steps.len());
    while let Some(step_state) = state.steps.get(cursor) {
        if !matches!(
            step_state.status,
            GraphStepStatus::Succeeded | GraphStepStatus::Skipped
        ) {
            break;
        }
        let Some(graph_step) = graph
            .steps
            .get(cursor)
            .filter(|step| step.id == step_state.step_id)
        else {
            break;
        };
        if let Some(group_id) = graph_step.fanout_group.as_deref()
            && !sync_points.iter().any(|sync| {
                sync.group_id.as_ref() == group_id
                    && sync.decision == runx_contracts::FanoutReceiptDecision::Proceed
            })
        {
            break;
        }
        cursor += 1;
    }
    cursor
}

fn run_positions(runs: &[StepRun]) -> BTreeMap<String, usize> {
    let mut positions = BTreeMap::new();
    for (index, run) in runs.iter().enumerate() {
        positions.insert(run.step_id.clone(), index);
    }
    positions
}

fn retry_budget_remaining(step: &GraphStep, attempt: u32) -> bool {
    let max_attempts = step.retry.as_ref().map_or(1, |retry| {
        u32::try_from(retry.max_attempts).unwrap_or(u32::MAX)
    });
    attempt < max_attempts
}

pub(super) fn reached_step_limit(
    initial: usize,
    current: usize,
    max_new_steps: Option<usize>,
) -> bool {
    max_new_steps.is_some_and(|max| current.saturating_sub(initial) >= max)
}

pub(super) fn enforce_guards(
    graph: &ExecutionGraph,
    step: &GraphStep,
    runs: &[StepRun],
) -> Result<(), RuntimeError> {
    let Some(policy) = &graph.policy else {
        return Ok(());
    };
    for gate in policy.guards.iter().filter(|gate| gate.step == step.id) {
        let Some(value) = transition_field_value(&gate.field, runs) else {
            return Err(RuntimeError::GraphBlocked {
                step_id: step.id.clone(),
                reason: format!("guard '{}' is unresolved", gate.field),
            });
        };
        if let Some(expected) = &gate.equals
            && value != expected
        {
            return Err(RuntimeError::GraphBlocked {
                step_id: step.id.clone(),
                reason: format!("guard '{}' expected {}", gate.field, display_json(expected)),
            });
        }
        if let Some(disallowed) = &gate.not_equals
            && value == disallowed
        {
            return Err(RuntimeError::GraphBlocked {
                step_id: step.id.clone(),
                reason: format!(
                    "guard '{}' must not equal {}",
                    gate.field,
                    display_json(disallowed)
                ),
            });
        }
        if gate.equals.is_none() && gate.not_equals.is_none() {
            return Err(RuntimeError::GraphBlocked {
                step_id: step.id.clone(),
                reason: format!("guard '{}' has no comparison", gate.field),
            });
        }
    }
    Ok(())
}

pub(super) fn transition_field_value<'a>(
    field: &str,
    runs: &'a [StepRun],
) -> Option<&'a JsonValue> {
    let mut segments = field.split('.');
    let step_id = segments.next()?;
    let run = runs.iter().rev().find(|run| run.step_id == step_id)?;
    let first = segments.next()?;
    // Guards and `when` conditions gate control flow, not data binding, so they may
    // reference diagnostic fields (notably `status`, to branch on a prior step's
    // success). Only the raw structured `skill_claim` blob is excluded here; the
    // stricter `BASE_OUTPUT_FIELDS` rejection applies to context EDGES (data inputs),
    // not to control-flow predicates.
    if first == "skill_claim" {
        return None;
    }
    let mut value = run.outputs.get(first)?;
    for segment in segments {
        let JsonValue::Object(object) = value else {
            return None;
        };
        value = object.get(segment)?;
    }
    Some(value)
}

pub(super) fn display_json(value: &JsonValue) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unprintable>".to_owned())
}

/// Resolve which steps a `when` condition selects out, given the runs so far.
/// A pending predicate source leaves the branch pending. A source already
/// selected out makes every branch depending on its absent output unreachable,
/// so selection propagates transitively instead of leaving the graph blocked.
pub(super) fn when_skipped_steps(
    graph: &ExecutionGraph,
    runs: &[StepRun],
    already_skipped: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeSet<String> {
    let mut skipped = already_skipped.clone();
    loop {
        let previous_len = skipped.len();
        for step in &graph.steps {
            let Some(when) = &step.when else {
                continue;
            };
            let predicate_step = when.field.split('.').next().unwrap_or_default();
            if skipped.contains(predicate_step) {
                skipped.insert(step.id.clone());
                continue;
            }
            let Some(value) = transition_field_value(&when.field, runs) else {
                continue;
            };
            let satisfied = match (&when.equals, &when.not_equals) {
                (Some(expected), _) => value == expected,
                (_, Some(disallowed)) => value != disallowed,
                _ => true,
            };
            if !satisfied {
                skipped.insert(step.id.clone());
            }
        }
        if skipped.len() == previous_len {
            return skipped;
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::collections::BTreeSet;

    use runx_contracts::{FanoutReceiptDecision, FanoutReceiptStrategy, FanoutReceiptSyncPoint};
    use runx_core::state_machine::{
        GraphStepStatus, SequentialGraphStepDefinition, create_sequential_graph_state,
    };
    use runx_parser::{ExecutionGraph, parse_graph_yaml, validate_graph};

    use super::{checkpoint_planning_cursor, terminal_prefix_cursor, when_skipped_steps};

    fn checkpoint_state(
        statuses: &[GraphStepStatus],
    ) -> (
        ExecutionGraph,
        runx_core::state_machine::SequentialGraphState,
    ) {
        let definitions = statuses
            .iter()
            .enumerate()
            .map(|(index, _)| SequentialGraphStepDefinition {
                id: format!("step_{index}"),
                context_from: None,
                retry: None,
                fanout_group: None,
            })
            .collect::<Vec<_>>();
        let mut state = create_sequential_graph_state("graph", &definitions);
        for (step, status) in state.steps.iter_mut().zip(statuses) {
            step.status = status.clone();
        }
        let steps = statuses
            .iter()
            .enumerate()
            .map(|(index, _)| format!("  - id: step_{index}\n    skill: ./noop\n"))
            .collect::<String>();
        let graph = validate_graph(
            parse_graph_yaml(&format!("name: graph\nsteps:\n{steps}"))
                .expect("checkpoint graph should parse"),
        )
        .expect("checkpoint graph should validate");
        (graph, state)
    }

    #[test]
    fn checkpoint_cursor_starts_at_the_first_non_terminal_step() {
        let (graph, state) = checkpoint_state(&[
            GraphStepStatus::Succeeded,
            GraphStepStatus::Skipped,
            GraphStepStatus::Failed,
            GraphStepStatus::Pending,
        ]);

        assert_eq!(
            checkpoint_planning_cursor(&graph, &state, &[]).expect("valid checkpoint"),
            2
        );
    }

    #[test]
    fn checkpoint_cursor_rejects_running_state_anywhere() {
        let (graph, state) = checkpoint_state(&[
            GraphStepStatus::Succeeded,
            GraphStepStatus::Pending,
            GraphStepStatus::Running,
        ]);

        let error = checkpoint_planning_cursor(&graph, &state, &[])
            .expect_err("running checkpoint must fail");
        assert!(
            error
                .to_string()
                .contains("checkpoint contains a running step")
        );
    }

    #[test]
    fn terminal_fanout_stays_at_sync_boundary_until_proceed_is_recorded() {
        let graph = validate_graph(
            parse_graph_yaml(
                r#"
name: checkpoint-fanout
fanout:
  groups:
    workers:
      strategy: all
      on_branch_failure: halt
steps:
  - id: first
    mode: fanout
    fanout_group: workers
    skill: ./noop
  - id: second
    mode: fanout
    fanout_group: workers
    skill: ./noop
  - id: finish
    skill: ./noop
"#,
            )
            .expect("fanout graph should parse"),
        )
        .expect("fanout graph should validate");
        let definitions = graph
            .steps
            .iter()
            .map(|step| SequentialGraphStepDefinition {
                id: step.id.clone(),
                context_from: None,
                retry: None,
                fanout_group: step.fanout_group.clone(),
            })
            .collect::<Vec<_>>();
        let mut state = create_sequential_graph_state(&graph.name, &definitions);
        state.steps[0].status = GraphStepStatus::Succeeded;
        state.steps[1].status = GraphStepStatus::Succeeded;

        assert_eq!(terminal_prefix_cursor(&graph, &state, &[], 0), 0);

        let sync = FanoutReceiptSyncPoint {
            group_id: "workers".into(),
            strategy: FanoutReceiptStrategy::All,
            decision: FanoutReceiptDecision::Proceed,
            rule_fired: "all_succeeded".into(),
            reason: "all branches succeeded".into(),
            branch_count: 2,
            success_count: 2,
            failure_count: 0,
            required_successes: 2,
            branch_receipts: Vec::new(),
            gate: None,
        };
        assert_eq!(terminal_prefix_cursor(&graph, &state, &[sync], 0), 2);
    }

    #[test]
    fn skipped_branch_predicates_propagate_to_unreachable_descendants() {
        let graph = validate_graph(
            parse_graph_yaml(
                r#"
name: conditional-propagation
steps:
  - id: source
    run:
      type: agent-task
      agent: test
      task: source
      outputs: { decision: string }
  - id: inspect
    when: { field: source.decision, equals: ready }
    run:
      type: agent-task
      agent: test
      task: inspect
      outputs: { decision: string }
  - id: reject
    when: { field: inspect.decision, equals: reject }
    run:
      type: agent-task
      agent: test
      task: reject
      outputs: { decision: string }
"#,
            )
            .expect("fixture graph should parse"),
        )
        .expect("fixture graph should validate");
        let skipped = when_skipped_steps(&graph, &[], &BTreeSet::from(["inspect".to_owned()]));

        assert!(skipped.contains("reject"));
    }
}
