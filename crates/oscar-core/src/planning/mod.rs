use crate::{
    config::{Capability, Config, RemoteReview, WorkMode},
    error::OscarError,
    routing,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
mod renderer;

macro_rules! classification {
    ($name:ident { $($value:ident),+ }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($value),+ }
    }
}
classification!(Difficulty {
    Trivial,
    Low,
    Medium,
    High,
    Critical
});
classification!(RiskLevel {
    Low,
    Medium,
    High,
    Critical
});
classification!(ProviderPreference { Local, Remote });
classification!(TaskKind {
    Explore,
    Implement,
    Test,
    Document,
    Reason,
    Review
});
classification!(ContextStrategy {
    Direct,
    Distilled,
    RepositorySlice
});
classification!(Confidence { Low, Medium, High });
impl ProviderPreference {
    pub fn key(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(pub String);

/// Built-in validators check artifact format, not correctness of generated code.
/// Registered host validators may supply stronger deterministic evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ValidationStrategy {
    NonEmpty,
    Json,
    Contains(String),
    Registered(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscalationPolicy {
    pub allow_remote: bool,
    pub max_local_attempts: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub description: String,
    pub kind: TaskKind,
    pub difficulty: Difficulty,
    pub risk: RiskLevel,
    pub preferred_provider: ProviderPreference,
    pub dependencies: Vec<TaskId>,
    pub required_capabilities: Vec<Capability>,
    pub context_requirements: Vec<String>,
    pub expected_outputs: Vec<String>,
    pub validation: ValidationStrategy,
    pub escalation_policy: EscalationPolicy,
    pub context_strategy: ContextStrategy,
    pub reason: String,
    /// Operator-set data boundary; overrides every mode, including full_remote.
    pub local_only_data: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub version: u32,
    pub goal: String,
    pub work_mode: WorkMode,
    pub tasks: Vec<Task>,
}
impl Plan {
    pub fn validate(&self, config: &Config) -> Result<(), OscarError> {
        config.validate()?;
        if self.version != 1 || self.work_mode != config.work_mode {
            return Err(OscarError::Plan(
                "unsupported version or work_mode differs from config".into(),
            ));
        }
        if self.goal.trim().is_empty()
            || self.goal.len() > config.limits.max_input_bytes / 2
            || self.tasks.is_empty()
            || self.tasks.len() > config.limits.max_tasks
        {
            return Err(OscarError::Plan(
                "empty/oversized goal or task count".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        for t in &self.tasks {
            if t.id.0.is_empty()
                || t.id.0.len() > 64
                || !t
                    .id
                    .0
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                || !ids.insert(t.id.clone())
            {
                return Err(OscarError::Plan("invalid or duplicate task ID".into()));
            }
            if t.title.trim().is_empty()
                || t.title.len() > 256
                || t.description.len() > config.limits.max_input_bytes / 2
                || t.description.trim().is_empty()
                || t.required_capabilities.is_empty()
                || t.required_capabilities.len() > 32
                || t.dependencies.len() > config.limits.max_tasks
                || t.expected_outputs.is_empty()
                || t.expected_outputs.len() > 32
                || t.context_requirements.len() > 32
                || t.reason.len() > 1024
                || t.expected_outputs
                    .iter()
                    .chain(&t.context_requirements)
                    .any(|s| s.len() > 1024)
                || !(1..=config.routing.max_local_attempts)
                    .contains(&t.escalation_policy.max_local_attempts)
            {
                return Err(OscarError::Plan(
                    "invalid task metadata or attempt limit".into(),
                ));
            }
            if matches!(&t.validation, ValidationStrategy::Contains(s) | ValidationStrategy::Registered(s) if s.is_empty() || s.len() > 256)
            {
                return Err(OscarError::Plan(
                    "invalid validator name or required text".into(),
                ));
            }
        }
        for t in &self.tasks {
            let deps: BTreeSet<_> = t.dependencies.iter().collect();
            if deps.len() != t.dependencies.len()
                || deps.contains(&t.id)
                || deps.iter().any(|d| !ids.contains(*d))
            {
                return Err(OscarError::Plan(
                    "duplicate, missing or self dependency".into(),
                ));
            }
        }
        self.waves()?;
        Ok(())
    }
    /// Stable topological waves. Execution additionally caps concurrency.
    pub fn waves(&self) -> Result<Vec<Vec<TaskId>>, OscarError> {
        let mut done = BTreeSet::new();
        let mut waves = Vec::new();
        while done.len() < self.tasks.len() {
            let wave: Vec<_> = self
                .tasks
                .iter()
                .filter(|t| {
                    !done.contains(&t.id) && t.dependencies.iter().all(|d| done.contains(d))
                })
                .map(|t| t.id.clone())
                .collect();
            if wave.is_empty() {
                return Err(OscarError::Plan("cyclic or invalid task graph".into()));
            }
            done.extend(wave.iter().cloned());
            waves.push(wave);
        }
        Ok(waves)
    }
    pub fn to_json(&self) -> Result<String, OscarError> {
        serde_json::to_string_pretty(self)
            .map_err(|_| OscarError::Plan("cannot serialize plan".into()))
    }
    pub fn from_json(text: &str, config: &Config) -> Result<Self, OscarError> {
        if text.len() > 2_097_152 {
            return Err(OscarError::Plan("plan exceeds 2 MiB".into()));
        }
        let p: Self =
            serde_json::from_str(text).map_err(|_| OscarError::Plan("invalid plan JSON".into()))?;
        p.validate(config)?;
        Ok(p)
    }
}

/// Transparent, conservative request heuristic; no model calls or repository access.
/// Applications can replace it with their own validated Plan.
pub fn plan_request(goal: &str, config: &Config) -> Result<Plan, OscarError> {
    config.validate()?;
    if goal.trim().is_empty() || goal.len() > config.limits.max_input_bytes / 2 {
        return Err(OscarError::Plan(
            "request is empty or exceeds half the input budget".into(),
        ));
    }
    let lower = goal.to_lowercase();
    let sensitive = [
        "auth",
        "jwt",
        "security",
        "cryptograph",
        "payment",
        "financial",
        "data loss",
        "migration",
        "concurren",
    ]
    .iter()
    .any(|s| lower.contains(s));
    let complex = sensitive
        || [
            "architect",
            "distributed",
            "deadlock",
            "root cause",
            "trade-off",
        ]
        .iter()
        .any(|s| lower.contains(s));
    let documentation = !complex
        && ["readme", "document", "summarize", "explain"]
            .iter()
            .any(|s| lower.contains(s));
    let risk = if sensitive {
        RiskLevel::High
    } else {
        RiskLevel::Low
    };
    let mut plan = Plan {
        version: 1,
        goal: goal.into(),
        work_mode: config.work_mode,
        tasks: Vec::new(),
    };
    let mut add = |id: &str,
                   title: &str,
                   description: &str,
                   kind,
                   difficulty,
                   cap,
                   deps: &[&str]|
     -> Result<(), OscarError> {
        let mut task = Task {
            id: TaskId(id.into()), title: title.into(), description: description.into(), kind, difficulty, risk,
            preferred_provider: ProviderPreference::Local, dependencies: deps.iter().map(|s| TaskId((*s).into())).collect(),
            required_capabilities: vec![cap], context_requirements: vec!["Goal and declared dependency artifacts; report missing repository evidence explicitly".into()],
            expected_outputs: vec![format!("{id}.md")], validation: ValidationStrategy::NonEmpty,
            escalation_policy: EscalationPolicy { allow_remote: true, max_local_attempts: config.routing.max_local_attempts },
            context_strategy: if config.routing.context_distillation { ContextStrategy::Distilled } else { ContextStrategy::Direct },
            reason: String::new(), local_only_data: false,
        };
        let decision = routing::select(config, &task, 0, goal.len() + description.len() + 256)?;
        task.preferred_provider = decision.provider;
        task.reason = decision.reason.into();
        plan.tasks.push(task);
        Ok(())
    };
    if documentation {
        add(
            "T01",
            "Produce documentation",
            "Produce a concise documentation artifact. State any missing source evidence.",
            TaskKind::Document,
            Difficulty::Low,
            Capability::Documentation,
            &[],
        )?;
        if config.routing.remote_review == RemoteReview::Always {
            add(
                "T02",
                "Review documentation",
                "Review the documentation artifact for unsupported claims and missing evidence.",
                TaskKind::Review,
                Difficulty::High,
                Capability::Review,
                &["T01"],
            )?;
        }
    } else {
        add(
            "T01",
            "Analyze available evidence",
            "Analyze the request and supplied evidence. Identify relevant interfaces and missing repository context; never claim files were inspected without evidence.",
            TaskKind::Explore,
            Difficulty::Medium,
            Capability::CodeAnalysis,
            &[],
        )?;
        if complex {
            add(
                "T02",
                "Choose design and constraints",
                "Use the analysis to resolve design trade-offs and risks. Record assumptions, constraints and validation criteria.",
                TaskKind::Reason,
                Difficulty::High,
                Capability::AdvancedReasoning,
                &["T01"],
            )?;
        }
        let dep = if complex { "T02" } else { "T01" };
        add(
            "T03",
            "Propose implementation",
            "Produce an implementation proposal or patch artifact using the design and evidence. Do not claim changes were applied.",
            TaskKind::Implement,
            Difficulty::Medium,
            Capability::CodeGeneration,
            &[dep],
        )?;
        add(
            "T04",
            "Propose validation cases",
            "Produce tests and expected outcomes for the design. Recommend deterministic checks; do not claim commands were executed.",
            TaskKind::Test,
            Difficulty::Low,
            Capability::Testing,
            &[dep],
        )?;
        if config.routing.remote_review == RemoteReview::Always
            || (sensitive && config.routing.remote_review == RemoteReview::Auto)
        {
            add(
                "T05",
                "Review sensitive proposals",
                "Review implementation and test artifacts against the design. Identify contradictions and unresolved security risks.",
                TaskKind::Review,
                Difficulty::High,
                Capability::Review,
                &["T03", "T04"],
            )?;
        }
    }
    plan.validate(config)?;
    Ok(plan)
}
