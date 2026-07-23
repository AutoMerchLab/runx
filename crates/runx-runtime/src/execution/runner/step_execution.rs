use std::path::Path;

use runx_parser::GraphStep;

use super::super::graph::{
    LoadedStepSkill, materialize_step_invocation_inputs,
    materialize_step_invocation_inputs_with_index,
};
use super::super::graph_index::PriorRunIndex;
use super::steps::{StepRunRequest, run_step_with_inputs};
use super::{Runtime, StepRun};
use crate::RuntimeError;
use crate::adapter::SkillAdapter;
use crate::host::Host;

pub(super) struct LoadedStepExecutionRequest<'a, A: SkillAdapter> {
    pub(super) runtime: &'a Runtime<A>,
    pub(super) graph_dir: &'a Path,
    pub(super) graph_name: &'a str,
    pub(super) step: &'a GraphStep,
    pub(super) attempt: u32,
    pub(super) loaded_skill: Option<LoadedStepSkill>,
    pub(super) host: &'a mut dyn Host,
}

pub(super) fn run_step_with_loaded_skill<A>(
    request: LoadedStepExecutionRequest<'_, A>,
    prior_runs: &[StepRun],
) -> Result<StepRun, RuntimeError>
where
    A: SkillAdapter,
{
    let inputs = materialize_step_invocation_inputs(request.step, prior_runs)?;
    run_step_with_loaded_skill_inputs(request, inputs)
}

pub(super) fn run_step_with_loaded_skill_index<A>(
    request: LoadedStepExecutionRequest<'_, A>,
    prior_run_index: &PriorRunIndex<'_>,
) -> Result<StepRun, RuntimeError>
where
    A: SkillAdapter,
{
    let inputs = materialize_step_invocation_inputs_with_index(request.step, prior_run_index)?;
    run_step_with_loaded_skill_inputs(request, inputs)
}

fn run_step_with_loaded_skill_inputs<A>(
    request: LoadedStepExecutionRequest<'_, A>,
    inputs: runx_contracts::JsonObject,
) -> Result<StepRun, RuntimeError>
where
    A: SkillAdapter,
{
    let LoadedStepExecutionRequest {
        runtime,
        graph_dir,
        graph_name,
        step,
        attempt,
        loaded_skill,
        host,
    } = request;
    if let Some(skill) = loaded_skill {
        return super::steps::run_step_with_loaded_skill_inputs(
            StepRunRequest {
                runtime,
                graph_dir,
                graph_name,
                step,
                attempt,
                inputs,
                host,
            },
            skill,
        );
    }
    run_step_with_inputs(runtime, graph_dir, graph_name, step, attempt, inputs, host)
}
