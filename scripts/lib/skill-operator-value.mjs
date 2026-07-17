import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";

import YAML from "yaml";

const TERMINAL_CAPABILITIES = new Set([
  "a2a",
  "catalog",
  "cli-tool",
  "external-skill",
  "external-adapter",
  "harness-hook",
  "http",
  "mcp",
  "thread-outbox-provider",
  "tool",
]);
const AGENT_TYPES = new Set(["agent", "agent-task"]);
const PROFILE_NAME = "X.yaml";

export function auditOfficialSkills(root) {
  const skillsRoot = path.join(root, "skills");
  const roots = readdirSync(skillsRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => path.join(skillsRoot, entry.name, PROFILE_NAME))
    .filter((profilePath) => existsSync(profilePath))
    .sort();
  const profiles = new Map();
  const references = new Map(roots.map((profilePath) => [profilePath, new Set()]));

  for (const profilePath of roots) {
    collectTopLevelReferences(profilePath, loadProfile(profilePath, profiles), roots, references);
  }

  return roots.map((profilePath) => {
    const profile = loadProfile(profilePath, profiles);
    const defaultTraversal = analyzeRunner(profilePath, profile, undefined, profiles, []);
    const traversal = analyzePackage(profilePath, profile, profiles);
    const proof = skillProof(root, path.dirname(profilePath), profilePath, profile, profiles);
    const catalog = profile.catalog ?? {};
    const capabilityBoundaries = [...traversal.terminals]
      .filter((type) => TERMINAL_CAPABILITIES.has(type))
      .sort();
    const incomingReferences = references.get(profilePath)?.size ?? 0;
    const issues = catalogIssues(catalog, traversal, capabilityBoundaries, proof);
    const improvements = catalogImprovements(catalog, traversal, capabilityBoundaries, proof);
    const disposition = skillDisposition({
      catalog,
      traversal,
      capabilityBoundaries,
      proof,
      incomingReferences,
    });
    return {
      skill: profile.skill ?? path.basename(path.dirname(profilePath)),
      path: relative(root, profilePath),
      visibility: catalog.visibility ?? "internal",
      role: catalog.role ?? "missing",
      audience: catalog.audience ?? "missing",
      execution: catalog.execution ?? null,
      completion: catalog.completion ?? null,
      requires_adapter: catalog.requires_adapter ?? null,
      approval: catalog.approval ?? null,
      default_runner: defaultTraversal.defaultRunner,
      default_terminal_types: [...defaultTraversal.terminals].sort(),
      default_capabilities: [...defaultTraversal.capabilities].sort(),
      default_managed_agent_acts: defaultTraversal.agentActs,
      terminal_types: [...traversal.terminals].sort(),
      capability_boundaries: capabilityBoundaries,
      capabilities: [...traversal.capabilities].sort(),
      managed_agent_acts: traversal.agentActs,
      artifact_outputs: traversal.artifactOutputs,
      approval_steps: traversal.approvals,
      graph_steps: traversal.graphSteps,
      incoming_references: incomingReferences,
      proof,
      issues,
      improvements,
      decision_status: "pending_review",
      disposition: disposition.value,
      rationale: disposition.rationale,
    };
  });
}

export function auditSummary(skills) {
  const byDisposition = countBy(skills, (skill) => skill.disposition);
  return {
    schema: "runx.skill_operator_value_audit.v1",
    skill_count: skills.length,
    public_count: skills.filter((skill) => skill.visibility === "public").length,
    public_context_count: skills.filter(
      (skill) => skill.visibility === "public" && skill.role === "context",
    ).length,
    managed_agent_count: skills.filter((skill) => skill.managed_agent_acts > 0).length,
    agent_only_default_count: skills.filter(
      (skill) => skill.managed_agent_acts > 0 && skill.capability_boundaries.length === 0,
    ).length,
    no_operation_proof_count: skills.filter((skill) => skill.proof.operation_cases === 0).length,
    no_semantic_proof_count: skills.filter((skill) => skill.proof.semantic_cases === 0).length,
    operator_journey_count: skills.reduce(
      (count, skill) => count + skill.proof.operator_journey_cases,
      0,
    ),
    no_contract_proof_count: skills.filter((skill) => skill.proof.total_cases === 0).length,
    pending_decision_count: skills.filter((skill) => skill.decision_status === "pending_review").length,
    dispositions: byDisposition,
  };
}

export function reviewSummary(skills, decisions, trials) {
  const audit = auditSummary(skills);
  return {
    skill_count: Object.keys(decisions.recommendations).length,
    current_skill_count: audit.skill_count,
    public_count: audit.public_count,
    internal_count: audit.skill_count - audit.public_count,
    managed_agent_count: audit.managed_agent_count,
    agent_only_default_count: audit.agent_only_default_count,
    no_operation_proof_count: audit.no_operation_proof_count,
    no_semantic_proof_count: audit.no_semantic_proof_count,
    operator_journey_count: audit.operator_journey_count,
    no_contract_proof_count: audit.no_contract_proof_count,
    locally_proven_count: trials.summary.locally_proven,
    locally_failed_count: trials.summary.failed,
    locally_unproven_count: trials.summary.unproven,
    full_bar_count: trials.summary.meets_full_bar,
    recommendations: countBy(
      Object.values(decisions.recommendations),
      (decision) => decision.action,
    ),
    archetypes: countBy(
      Object.values(decisions.recommendations),
      (decision) => decision.archetype,
    ),
  };
}

export function reviewDocument(skills, decisions, trials) {
  const summary = reviewSummary(skills, decisions, trials);
  const trialsBySkill = new Map(trials.skills.map((trial) => [trial.skill, trial]));
  const lines = [
    "# Core Skill Product Review",
    "",
    "Generated by `scripts/audit-skill-operator-value.mjs`. Do not edit by hand.",
    "",
    `**Status: ${decisions.status}.** The review covers all ${summary.skill_count} top-level skill packages.`,
    `The tree contains ${summary.current_skill_count} core packages: ${summary.public_count} public and ${summary.internal_count} internal.`,
    "No additional package is removed or hidden by this review. Improvement recommendations preserve the capability until a separate product decision approves a migration.",
    "",
    "## Product bar",
    "",
    "A core skill earns its place by providing specialized workflow, domain expertise, reusable resources, governed execution, or a durable artifact that materially improves what an agent can do. Deterministic execution is one valid shape, not the definition of skill value.",
    "",
    "1. **Runtime and provider operations** execute a real boundary and prove the effect with runtime or provider readback.",
    "2. **Domain and operator workflows** encode non-obvious procedure, structured judgment, gates, handoffs, and recovery across a recurring job.",
    "3. **Artifact and distribution skills** produce durable, provenance-bound research, content, security, growth, or ecosystem artifacts with review and publication handoffs.",
    "4. **Builder skills** make designing, testing, improving, packaging, or distributing skills materially easier and strengthen the Runx ecosystem flywheel.",
    "5. **Context skills** create reusable bounded packets that improve downstream work without claiming an external mutation.",
    "",
    "Every archetype must have a truthful closure, explicit authority and stop conditions, a declared artifact or effect, and replayable proof appropriate to its claim. Provider execution needs real readback. Agent-authored work needs a stable output contract and realistic harness or forward-test evidence.",
    "",
    "Provider operations follow one evidence contract: runtime-resolved credentials, pre-call scope and idempotency checks, approval only at an external mutation, provider execution through a declared boundary, acknowledgement kept separate from finality, and stable-id readback before a terminal effect claim. The keyless NWS lane is the reference read shape; Nitrosend is the reference credentialed account-operation shape.",
    "",
    "Managed-agent execution is optional infrastructure, not an admission category. Agent acts yield `needs_agent` by default. In-process execution requires per-run `--managed-agent` consent and a visible round budget; configured credentials are availability, not consent.",
    "Prepared context remains digest-bound and drift-checked for every run, but human context approval is reserved for selected graphs that declare mutation. Safe reads, analysis, planning, and artifact generation proceed without that gate.",
    "",
    "The normative detail lives in",
    "[Skill Quality Standard](skill-quality-standard.md).",
    "",
    "## Implemented authoring consolidation",
    "",
    "Five overlapping or under-owned package names were retired after their useful behavior moved into canonical owners:",
    "",
    "- `design-skill` moved into the read-only `skill-lab design` runner.",
    "- `write-harness` moved into `skill-lab harness`, with path validation and native replay.",
    "- `improve-skill` moved into `skill-lab improve`, preserving evidence-led diagnosis, bounded writes, and regression coverage.",
    "- `skill-testing` moved into `review-skill`, which now owns native inspection, safe harness execution, and evidence-bounded assessment.",
    "- `evolve` was retired because its shipped surface stopped at shallow repository inspection plus spec planning; use `work-plan` for bounded planning, `skill-lab` for skill changes, and the named downstream execution lane for mutation.",
    "",
    "The official lock, generated Rust catalog, packet schemas, documentation, and consumers now point only at the canonical owners. No generic host authoring guidance was copied into Runx: a host `skill-creator` may guide the agent, while `skill-lab` remains the only portable Runx implementation.",
    "",
    "## How this review was performed",
    "",
    "- **Static execution audit:** followed each top-level `X.yaml` default runner",
    "  transitively and recorded agent acts, capability boundaries, artifact declarations, metadata gaps, consumers, and fixtures.",
    "- **Archetype-aware proof:** operation fixtures prove effects; supplied-answer harnesses prove agent-artifact contracts without spending model tokens; provider evidence remains separate and read-only.",
    "- **Product-role review:** checked the root skill contract, operator architecture, first-party catalog map, public registry evidence, distribution value, and complete workflow ownership.",
    "- **Removal guard:** a weak implementation becomes an improvement recommendation unless canonical ownership and consumer migration prove the package redundant.",
    "",
    "## Current evidence",
    "",
    `- Top-level core packages: ${summary.current_skill_count}`,
    `- Public packages: ${summary.public_count}; internal packages: ${summary.internal_count}`,
    `- Packages containing agent work: ${summary.managed_agent_count}`,
    `- Agent-only default closures: ${summary.agent_only_default_count}`,
    `- Packages without operation proof: ${summary.no_operation_proof_count}`,
    `- Packages without semantic output proof: ${summary.no_semantic_proof_count}`,
    `- Replayable multi-step operator journeys: ${summary.operator_journey_count}`,
    `- Packages without any replayable contract proof: ${summary.no_contract_proof_count}`,
    `- Public harness trials: ${summary.locally_proven_count} passed, ${summary.locally_failed_count} failed, ${summary.locally_unproven_count} unproven`,
    `- Public packages currently meeting their complete archetype bar: ${summary.full_bar_count}`,
    `- Recommendations: ${Object.entries(summary.recommendations).map(([key, value]) => `${key}=${value}`).join(", ")}`,
    `- Archetypes: ${Object.entries(summary.archetypes).map(([key, value]) => `${key}=${value}`).join(", ")}`,
    "",
    "## Recommendation meanings",
    "",
    "- `keep`: the package has a clear core role and evidence appropriate to its current claim.",
    "- `improve`: keep the package core and close the named execution, artifact, metadata, proof, or provider gap.",
    "- `consolidate_review`: compare overlapping packages as one product pipeline; preserve all names until a canonical owner and migration are explicitly approved.",
    "- `internal_fixture`: retain non-public deterministic test packages used to prove canonical parent skills.",
    "- `internal_runtime`: retain non-public provider execution rails behind their canonical parent skills.",
    "",
    "## Package-by-package review",
    "",
    "| Skill | Archetype | Catalog role | Default execution shape | Evidence | Decision | Rationale | Improvement |",
    "|---|---|---|---|---|---|---|---|",
  ];
  const skillsByName = new Map(skills.map((skill) => [skill.skill, skill]));
  for (const [name, decision] of Object.entries(decisions.recommendations).sort(([left], [right]) => left.localeCompare(right))) {
    const skill = skillsByName.get(name);
    const trial = trialsBySkill.get(name);
    const closure = executionShape(skill);
    const evidence = trialEvidence(skill, trial);
    lines.push(
      `| ${escapeCell(name)} | ${decision.archetype} | ${skill.visibility}/${skill.role} | ${escapeCell(closure)} | ${escapeCell(evidence)} | ${decision.action} | ${escapeCell(decision.reason)} | ${escapeCell(decision.improvement ?? "none")} |`,
    );
  }
  lines.push(
    "",
    "## Consolidation and removal guard",
    "",
    "No remaining recommendation in this review authorizes deletion, relocation, or public-to-internal demotion. A future removal must identify the canonical replacement, prove consumer and registry migration, preserve useful artifacts and history, and receive explicit product approval before the tree changes.",
    "",
  );
  return lines.join("\n");
}

function executionShape(skill) {
  const boundaries = skill.default_capabilities.length > 0
    ? skill.default_capabilities.join(", ")
    : skill.default_terminal_types.filter((type) => type !== "agent" && type !== "agent-task").join(", ");
  if (skill.default_managed_agent_acts > 0 && boundaries) {
    return `${boundaries}; ${skill.default_managed_agent_acts} agent act(s)`;
  }
  if (skill.default_managed_agent_acts > 0) {
    return `${skill.default_managed_agent_acts} agent act(s) -> declared artifact`;
  }
  return boundaries || "unresolved";
}

function trialEvidence(skill, trial) {
  if (!trial) return `internal; ${skill.issues.length} blocking finding(s); not trialled`;
  if (trial.meets_full_bar) return "complete archetype bar";
  const parts = [`harness ${trial.local_trial}`, `${trial.static_findings.length} blocking finding(s)`];
  if (skill.proof.operation_cases > 0) parts.push(`${skill.proof.operation_cases} operation proof(s)`);
  if (skill.proof.agent_contract_cases > 0) parts.push(`${skill.proof.agent_contract_cases} agent-contract proof(s)`);
  if (skill.proof.operator_journey_cases > 0) parts.push(`${skill.proof.operator_journey_cases} operator journey(s)`);
  if (trial.provider_readback === "passed") parts.push("provider readback passed");
  else if (trial.provider_readback === "not_proven_by_isolated_fixture") {
    parts.push("provider readback unproven");
  }
  return parts.join("; ");
}

export function semanticCapabilityFindings(profile, label, options = {}) {
  const syntheticPath = options.profilePath ?? path.resolve("/virtual", label, PROFILE_NAME);
  const profiles = new Map([[syntheticPath, profile]]);
  for (const [profilePath, nestedProfile] of options.profiles ?? []) {
    profiles.set(path.resolve(profilePath), nestedProfile);
  }
  const traversal = analyzePackage(syntheticPath, profile, profiles);
  const capabilityBoundaries = [...traversal.terminals].filter((type) => TERMINAL_CAPABILITIES.has(type));
  const proof = options.proof ?? { total_cases: 1, operation_cases: 1, agent_contract_cases: 1 };
  return catalogIssues(profile.catalog ?? {}, traversal, capabilityBoundaries, proof)
    .map((finding) => `${label}: ${finding}`);
}

function analyzePackage(profilePath, profile, profiles) {
  const result = emptyTraversal("all");
  const runnerNames = Object.keys(profile.runners ?? {}).sort();
  if (runnerNames.length === 0) {
    result.errors.add(`no runners declared in ${relative(process.cwd(), profilePath)}`);
    return result;
  }
  for (const runnerName of runnerNames) {
    mergeTraversal(
      result,
      analyzeRunner(profilePath, profile, runnerName, profiles, []),
    );
  }
  return result;
}

function analyzeRunner(profilePath, profile, requestedRunner, profiles, stack) {
  const runners = profile.runners ?? {};
  const selection = selectRunner(runners, requestedRunner);
  const result = emptyTraversal(selection.name);
  if (selection.error) {
    result.errors.add(selection.error);
    return result;
  }
  const key = `${profilePath}#${selection.name}`;
  if (stack.includes(key)) {
    result.errors.add(`cyclic runner reference: ${[...stack, key].join(" -> ")}`);
    return result;
  }
  visitExecution(selection.runner, profilePath, profiles, [...stack, key], result);
  return result;
}

function visitExecution(execution, profilePath, profiles, stack, result, attachedArtifacts) {
  if (!execution || typeof execution !== "object") {
    result.errors.add(`missing execution object in ${relative(process.cwd(), profilePath)}`);
    return;
  }
  const type = execution.type ?? execution.source?.type ?? (execution.graph ? "graph" : undefined);
  if (type === "graph") {
    visitGraph(execution.graph, profilePath, profiles, stack, result);
    return;
  }
  if (AGENT_TYPES.has(type)) {
    result.terminals.add(type);
    result.agentActs += 1;
    if (declaresArtifact(execution.artifacts) || declaresArtifact(attachedArtifacts)) {
      result.artifactOutputs += 1;
    }
    return;
  }
  if (type === "approval") {
    result.terminals.add(type);
    result.approvals += 1;
    return;
  }
  if (typeof type === "string") {
    result.terminals.add(type);
    if (TERMINAL_CAPABILITIES.has(type)) result.capabilities.add(executionCapability(type, execution));
    return;
  }
  result.errors.add(`execution type is unresolved in ${relative(process.cwd(), profilePath)}`);
}

function visitGraph(graph, profilePath, profiles, stack, result) {
  const steps = Array.isArray(graph?.steps) ? graph.steps : [];
  if (steps.length === 0) {
    result.errors.add(`graph has no steps in ${relative(process.cwd(), profilePath)}`);
    return;
  }
  for (const step of steps) {
    result.graphSteps += 1;
    if (typeof step?.skill === "string") {
      visitSkillReference(step.skill, step.runner, profilePath, profiles, stack, result);
    } else if (typeof step?.tool === "string") {
      result.terminals.add("tool");
      result.capabilities.add(`tool:${step.tool}`);
    } else if (step?.run) {
      visitExecution(step.run, profilePath, profiles, stack, result, step.artifacts);
    } else {
      result.errors.add(`graph step '${step?.id ?? "unknown"}' has no execution`);
    }
  }
}

function visitSkillReference(reference, runner, profilePath, profiles, stack, result) {
  const resolved = resolveSkillProfile(profilePath, reference, profiles);
  if (!resolved) {
    if (isExternalSkillReference(profilePath, reference)) {
      result.terminals.add("external-skill");
      result.capabilities.add(`skill:${reference}`);
    } else {
      result.errors.add(`skill reference '${reference}' does not resolve from ${relative(process.cwd(), profilePath)}`);
    }
    return;
  }
  let profile;
  try {
    profile = loadProfile(resolved, profiles);
  } catch (error) {
    result.errors.add(error.message);
    return;
  }
  const nested = analyzeRunner(resolved, profile, runner, profiles, stack);
  mergeTraversal(result, nested);
}

function resolveSkillProfile(profilePath, reference, profiles = new Map()) {
  if (reference.startsWith("registry:") || reference.includes("@")) return null;
  const base = path.dirname(profilePath);
  const candidate = path.resolve(base, reference);
  const paths = [
    candidate,
    path.join(candidate, PROFILE_NAME),
    path.join(base, `${reference}.yaml`),
  ];
  for (const value of paths) {
    const absoluteValue = path.resolve(value);
    if (path.basename(absoluteValue) === PROFILE_NAME && (existsSync(absoluteValue) || profiles.has(absoluteValue))) {
      return absoluteValue;
    }
    if ((existsSync(absoluteValue) || profiles.has(absoluteValue)) && path.extname(absoluteValue) === ".yaml") {
      return absoluteValue;
    }
  }
  return null;
}

function selectRunner(runners, requestedRunner) {
  if (requestedRunner) {
    return runners[requestedRunner]
      ? { name: requestedRunner, runner: runners[requestedRunner] }
      : { name: requestedRunner, error: `runner '${requestedRunner}' is not declared` };
  }
  const entries = Object.entries(runners);
  const defaults = entries.filter(([, runner]) => runner?.default === true);
  if (defaults.length === 1) return { name: defaults[0][0], runner: defaults[0][1] };
  if (defaults.length > 1) return { name: "unresolved", error: "multiple default runners" };
  if (entries.length === 1) return { name: entries[0][0], runner: entries[0][1] };
  return { name: "unresolved", error: "no default runner" };
}

function emptyTraversal(defaultRunner) {
  return {
    defaultRunner,
    terminals: new Set(),
    capabilities: new Set(),
    errors: new Set(),
    agentActs: 0,
    artifactOutputs: 0,
    approvals: 0,
    graphSteps: 0,
  };
}

function mergeTraversal(target, source) {
  for (const terminal of source.terminals) target.terminals.add(terminal);
  for (const capability of source.capabilities) target.capabilities.add(capability);
  for (const error of source.errors) target.errors.add(error);
  target.agentActs += source.agentActs;
  target.artifactOutputs += source.artifactOutputs;
  target.approvals += source.approvals;
  target.graphSteps += source.graphSteps;
}

function catalogIssues(catalog, traversal, capabilityBoundaries, proof) {
  const issues = [...traversal.errors];
  const isPublic = (catalog.visibility ?? "internal") === "public";
  if (!isPublic) return issues.sort();
  if (catalog.requires_adapter === true && !capabilityBoundaries.some(isAdapterBoundary)) {
    issues.push("catalog requires_adapter claim has no reachable adapter boundary");
  }
  if (traversal.agentActs > 0 && traversal.artifactOutputs === 0) {
    issues.push("agent-authored closure has no declared artifact packet");
  }
  if (proof.total_cases === 0) {
    issues.push("public skill has no executable contract or operation proof");
  }
  return [...new Set(issues)].sort();
}

function catalogImprovements(catalog, traversal, capabilityBoundaries, proof) {
  const improvements = [];
  const isPublic = (catalog.visibility ?? "internal") === "public";
  if (!isPublic) return improvements;
  if (!hasCompleteCapabilityMetadata(catalog)) {
    improvements.push("declare execution, completion, adapter, and approval metadata");
  }
  if (capabilityBoundaries.length > 0 && proof.operation_cases === 0) {
    improvements.push("add standalone operation-boundary proof");
  }
  if (traversal.agentActs > 0 && proof.agent_contract_cases === 0) {
    improvements.push("add replayable agent-artifact contract proof");
  }
  if (proof.semantic_cases === 0) {
    improvements.push("replace status-only smoke coverage with a semantic output oracle");
  }
  return [...new Set(improvements)].sort();
}

function skillDisposition({ catalog, traversal, capabilityBoundaries, proof, incomingReferences }) {
  const visibility = catalog.visibility ?? "internal";
  if (visibility !== "public") {
    return { value: "keep", rationale: "Internal package; retain as owned implementation or context." };
  }
  if (traversal.errors.size > 0) {
    return { value: "improve", rationale: "Default execution closure is unresolved or cyclic." };
  }
  if (catalog.requires_adapter === true && !capabilityBoundaries.some(isAdapterBoundary)) {
    return { value: "improve", rationale: "Declared adapter ownership does not match the reachable closure." };
  }
  if (traversal.agentActs > 0 && traversal.artifactOutputs === 0) {
    return { value: "improve", rationale: "Agent work needs a declared durable artifact contract." };
  }
  if (proof.total_cases === 0) {
    return { value: "improve", rationale: "Public skill needs replayable proof for its declared closure." };
  }
  if (!hasCompleteCapabilityMetadata(catalog)) {
    return { value: "improve", rationale: "Public execution metadata is incomplete." };
  }
  if (capabilityBoundaries.length > 0 && proof.operation_cases === 0) {
    return { value: "improve", rationale: "Operation boundary exists but lacks standalone operation proof." };
  }
  if (traversal.agentActs > 0 && proof.agent_contract_cases === 0) {
    return { value: "improve", rationale: "Agent-authored artifact lacks replayable contract proof." };
  }
  if (proof.semantic_cases === 0) {
    return { value: "improve", rationale: "Status-only replay does not prove the operator-visible result." };
  }
  return { value: "keep", rationale: "Declared closure has matching replayable proof." };
}

function skillProof(root, skillDir, profilePath, profile, profiles) {
  const fixtureDir = path.join(skillDir, "fixtures");
  const fixtureFiles = existsSync(fixtureDir) ? recursiveFiles(fixtureDir) : [];
  let standaloneCases = 0;
  let operationCases = 0;
  let agentContractCases = 0;
  let semanticCases = 0;
  let operatorJourneyCases = 0;
  let rejectedCases = 0;
  const cases = [];
  for (const fixturePath of fixtureFiles) {
    if (!/\.(json|ya?ml)$/u.test(fixturePath)) continue;
    try {
      const fixture = parseDocument(fixturePath);
      const proofCase = executableSkillCase(fixture, profilePath, profile, profiles, {
        casePath: fixturePath,
      });
      if (proofCase) {
        standaloneCases += 1;
        if (proofCase.operation) operationCases += 1;
        if (proofCase.agent_contract) agentContractCases += 1;
        if (proofCase.semantic) semanticCases += 1;
        if (proofCase.operator_journey) operatorJourneyCases += 1;
        cases.push({
          kind: "fixture",
          name: fixture.name ?? path.basename(fixturePath),
          path: relative(root, fixturePath),
          runner: fixture.runner ?? null,
          ...proofCase,
        });
      } else rejectedCases += 1;
    } catch {
      // Other fixture documents may belong to provider protocols. They are not
      // admitted as operator-value proof until the trial runner names them.
    }
  }
  const harnessCases = Array.isArray(profile.harness?.cases) ? profile.harness.cases : [];
  let inlineCases = 0;
  const suppliedAnswerCases = harnessCases.filter(hasCallerAnswers).length;
  for (const entry of harnessCases) {
    const proofCase = executableSkillCase(entry, profilePath, profile, profiles, { embedded: true });
    if (!proofCase) continue;
    inlineCases += 1;
    if (proofCase.operation) operationCases += 1;
    if (proofCase.agent_contract) agentContractCases += 1;
    if (proofCase.semantic) semanticCases += 1;
    if (proofCase.operator_journey) operatorJourneyCases += 1;
    cases.push({
      kind: "inline",
      name: entry.name ?? "unnamed-inline-case",
      path: relative(root, skillDir),
      runner: entry.runner ?? null,
      ...proofCase,
    });
  }
  return {
    standalone_cases: standaloneCases,
    inline_cases: inlineCases,
    supplied_answer_cases: suppliedAnswerCases,
    operation_cases: operationCases,
    agent_contract_cases: agentContractCases,
    semantic_cases: semanticCases,
    operator_journey_cases: operatorJourneyCases,
    total_cases: standaloneCases + inlineCases,
    rejected_cases: rejectedCases + harnessCases.length - inlineCases,
    real_cases: semanticCases,
    cases,
  };
}

function executableSkillCase(value, profilePath, profile, profiles, options = {}) {
  if (!value || typeof value !== "object") return null;
  if (!options.embedded && value.kind !== "skill" && value.kind !== "graph") return null;
  if (!value.expect || typeof value.expect.status !== "string") return null;
  const traversal = proofCaseTraversal(value, profilePath, profile, profiles, options);
  if (traversal.errors.size > 0) return null;
  const suppliedAnswers = hasCallerAnswers(value);
  const explicitSemanticOracle = Boolean(
    value.expect.output
    || (value.expect.step_outputs && Object.keys(value.expect.step_outputs).length > 0),
  );
  const semantic = suppliedAnswers || explicitSemanticOracle;
  const operatorJourney = traversal.graphSteps > 1
    && Array.isArray(value.expect.steps)
    && value.expect.steps.length > 1
    && explicitSemanticOracle;
  const agentContract = traversal.agentActs > 0 && suppliedAnswers;
  const operation = semantic
    && (traversal.agentActs === 0 || operatorJourney)
    && [...traversal.terminals].some((type) => TERMINAL_CAPABILITIES.has(type));
  return {
    proof_type: operatorJourney
      ? "operator_journey"
      : operation
        ? "operation"
        : agentContract
          ? "agent_contract"
          : "structural",
    supplied_answers: suppliedAnswers,
    semantic,
    operation,
    agent_contract: agentContract,
    operator_journey: operatorJourney,
    provider_readback: value.metadata?.source_case === "live-keyless-read"
      ? "live-keyless-read"
      : null,
  };
}

function proofCaseTraversal(value, profilePath, profile, profiles, options) {
  if (value.kind !== "graph" || typeof value.target !== "string" || !options.casePath) {
    return analyzeRunner(profilePath, profile, value.runner, profiles, []);
  }
  const graphPath = path.resolve(path.dirname(options.casePath), value.target);
  if (!existsSync(graphPath)) {
    const traversal = emptyTraversal("fixture-graph");
    traversal.errors.add(`graph fixture target does not resolve: ${relative(process.cwd(), graphPath)}`);
    return traversal;
  }
  const graph = parseDocument(graphPath);
  const fixtureProfile = {
    runners: {
      fixture: {
        default: true,
        type: "graph",
        graph,
      },
    },
  };
  return analyzeRunner(graphPath, fixtureProfile, "fixture", profiles, []);
}

function collectTopLevelReferences(profilePath, profile, roots, references) {
  visitValues(profile, (key, value) => {
    if (key !== "skill" || typeof value !== "string") return;
    const resolved = resolveSkillProfile(profilePath, value);
    if (!resolved) return;
    const owner = roots.find((rootPath) => resolved === rootPath || resolved.startsWith(`${path.dirname(rootPath)}${path.sep}`));
    if (owner && owner !== profilePath) references.get(owner)?.add(profilePath);
  });
}

function visitValues(value, visit, key = undefined) {
  if (Array.isArray(value)) {
    for (const entry of value) visitValues(entry, visit);
    return;
  }
  if (!value || typeof value !== "object") return;
  for (const [childKey, childValue] of Object.entries(value)) {
    visit(childKey, childValue, key);
    visitValues(childValue, visit, childKey);
  }
}

function loadProfile(profilePath, profiles) {
  const absolutePath = path.resolve(profilePath);
  if (profiles.has(absolutePath)) return profiles.get(absolutePath);
  if (!existsSync(absolutePath)) throw new Error(`skill profile does not exist: ${absolutePath}`);
  const profile = YAML.parse(readFileSync(absolutePath, "utf8"));
  profiles.set(absolutePath, profile);
  return profile;
}

function parseDocument(filePath) {
  const source = readFileSync(filePath, "utf8");
  return filePath.endsWith(".json") ? JSON.parse(source) : YAML.parse(source);
}

function hasCallerAnswers(value) {
  return Boolean(value?.caller?.answers && Object.keys(value.caller.answers).length > 0);
}

function declaresArtifact(artifacts) {
  if (!artifacts || typeof artifacts !== "object") return false;
  return Boolean(
    artifacts.wrap_as
    || artifacts.packet
    || Object.keys(artifacts.named_emits ?? {}).length > 0
    || Object.keys(artifacts.packets ?? {}).length > 0,
  );
}

function recursiveFiles(root) {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const child = path.join(root, entry.name);
    return entry.isDirectory() ? recursiveFiles(child) : [child];
  });
}

function isAdapterBoundary(type) {
  return !["cli-tool", "harness-hook"].includes(type);
}

function hasCompleteCapabilityMetadata(catalog) {
  return Boolean(
    catalog.execution
    && catalog.completion
    && catalog.requires_adapter !== undefined
    && catalog.approval,
  );
}

function executionCapability(type, execution) {
  const target = execution.tool
    ?? execution.command
    ?? execution.adapter
    ?? execution.url
    ?? execution.source?.tool
    ?? execution.source?.server?.command;
  return target ? `${type}:${target}` : type;
}

function isExternalSkillReference(profilePath, reference) {
  if (reference.startsWith("registry:") || reference.startsWith("@")) return true;
  const candidate = path.resolve(path.dirname(profilePath), reference);
  return existsSync(path.join(candidate, "SKILL.md"));
}

function countBy(values, keyFor) {
  return Object.fromEntries(
    [...values.reduce((counts, value) => {
      const key = keyFor(value);
      counts.set(key, (counts.get(key) ?? 0) + 1);
      return counts;
    }, new Map())].sort(([left], [right]) => left.localeCompare(right)),
  );
}

function escapeCell(value) {
  return String(value).replaceAll("|", "\\|").replaceAll("\n", " ");
}

function code(value) {
  return `\`${value}\``;
}

function relative(root, value) {
  return path.relative(root, value).split(path.sep).join("/");
}
