use runx_contracts::{
    JsonNumber, JsonObject, JsonValue, SkillArchitectureDecision, SkillExecutionLane,
};
use runx_parser::{SourceKind, ValidatedSkillPackage};

use super::super::path::invalid_skill_change;
use crate::RuntimeError;

#[derive(Default)]
pub(super) struct CandidateResourceUsage {
    pub(super) max_fanout: u64,
    pub(super) process_spawns: u64,
    pub(super) network: bool,
    pub(super) domain_modules: bool,
}

impl CandidateResourceUsage {
    pub(super) fn as_json(&self) -> JsonValue {
        JsonValue::Object(JsonObject::from([
            (
                "max_fanout".to_owned(),
                JsonValue::Number(JsonNumber::U64(self.max_fanout)),
            ),
            (
                "process_spawns".to_owned(),
                JsonValue::Number(JsonNumber::U64(self.process_spawns)),
            ),
            ("network".to_owned(), JsonValue::Bool(self.network)),
            (
                "domain_modules".to_owned(),
                JsonValue::Bool(self.domain_modules),
            ),
        ]))
    }
}

pub(super) fn candidate_resource_usage(package: &ValidatedSkillPackage) -> CandidateResourceUsage {
    let mut usage = CandidateResourceUsage {
        max_fanout: 1,
        domain_modules: !package.javascript_modules.is_empty(),
        ..CandidateResourceUsage::default()
    };
    for profile in package.profiles.values() {
        for runner in profile.runners.values() {
            classify_runner(runner, &mut usage);
        }
    }
    if usage.domain_modules {
        usage.process_spawns = usage.process_spawns.saturating_add(1);
    }
    usage
}

fn classify_runner(
    runner: &runx_parser::SkillRunnerDefinition,
    usage: &mut CandidateResourceUsage,
) {
    classify_source(
        runner.source.source_type,
        runner
            .source
            .sandbox
            .as_ref()
            .and_then(|sandbox| sandbox.network)
            .unwrap_or(false),
        usage,
    );
    let Some(graph) = &runner.source.graph else {
        return;
    };
    for group in graph.fanout_groups.keys() {
        let branches = graph
            .steps
            .iter()
            .filter(|step| step.fanout_group.as_deref() == Some(group.as_str()))
            .count() as u64;
        usage.max_fanout = usage.max_fanout.max(branches);
    }
    for step in &graph.steps {
        classify_graph_step(step, usage);
    }
}

fn classify_graph_step(step: &runx_parser::GraphStep, usage: &mut CandidateResourceUsage) {
    classify_scopes(&step.scopes, usage);
    if let Some(tool) = &step.tool {
        classify_native_tool(tool, usage);
    }
    let Some(run) = &step.run else {
        return;
    };
    let Some(source) = run.source() else {
        return;
    };
    let network = source
        .sandbox
        .as_ref()
        .and_then(|sandbox| sandbox.network)
        .unwrap_or(false);
    classify_source(source.source_type, network, usage);
}

fn classify_scopes(scopes: &[String], usage: &mut CandidateResourceUsage) {
    usage.network |= scopes.iter().any(|scope| scope == "net:process");
}

fn classify_source(kind: SourceKind, network: bool, usage: &mut CandidateResourceUsage) {
    classify_source_name(kind.as_str(), network, usage);
}

fn classify_source_name(kind: &str, network: bool, usage: &mut CandidateResourceUsage) {
    match kind {
        "javascript" => usage.domain_modules = true,
        "cli-tool" | "mcp" | "external-adapter" | "thread-outbox-provider" => {
            usage.process_spawns = usage.process_spawns.saturating_add(1);
        }
        "a2a" => usage.network = true,
        _ => {}
    }
    usage.network |= network;
}

fn classify_native_tool(tool: &str, usage: &mut CandidateResourceUsage) {
    if tool == "command.execute" {
        usage.process_spawns = usage.process_spawns.saturating_add(1);
    }
    if tool.starts_with("http.") || tool.starts_with("provider.") || tool == "web.fetch" {
        usage.network = true;
    }
}

pub(super) fn validate_architecture_resources(
    architecture: &SkillArchitectureDecision,
    usage: &CandidateResourceUsage,
) -> Result<(), RuntimeError> {
    let budget = &architecture.resource_budget;
    if usage.max_fanout > budget.max_fanout {
        return Err(invalid_skill_change(format!(
            "candidate fan-out {} exceeds architecture budget {}",
            usage.max_fanout, budget.max_fanout
        )));
    }
    if usage.process_spawns > budget.max_process_spawns {
        return Err(invalid_skill_change(format!(
            "candidate process count {} exceeds architecture budget {}",
            usage.process_spawns, budget.max_process_spawns
        )));
    }
    if usage.network && !budget.network_allowed {
        return Err(invalid_skill_change(
            "candidate declares a network lane but its architecture forbids network access",
        ));
    }
    let planned_domain_module = architecture
        .required_behaviors
        .iter()
        .any(|behavior| behavior.lane == SkillExecutionLane::DomainModule);
    if usage.domain_modules != planned_domain_module {
        return Err(invalid_skill_change(
            "candidate domain modules must exactly match the architecture decision",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CandidateResourceUsage, classify_native_tool, classify_scopes};

    #[test]
    fn supplied_evidence_indexing_does_not_claim_network_access() {
        let mut usage = CandidateResourceUsage::default();

        classify_native_tool("evidence.index_fetch_sources", &mut usage);

        assert!(!usage.network);
    }

    #[test]
    fn actual_network_tools_claim_network_access() {
        for tool in ["http.read", "provider.read", "web.fetch"] {
            let mut usage = CandidateResourceUsage::default();
            classify_native_tool(tool, &mut usage);
            assert!(usage.network, "{tool} must consume the network budget");
        }
    }

    #[test]
    fn process_network_scope_claims_network_access() {
        let mut usage = CandidateResourceUsage::default();
        let scopes = vec!["net:process".to_owned()];

        classify_scopes(&scopes, &mut usage);

        assert!(usage.network);
    }
}
