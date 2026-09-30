use super::*;
use std::fmt::Write;

impl Plan {
    /// Render only from the canonical plan; no independent model-generated Markdown.
    pub fn render_markdown(&self) -> Result<String, OscarError> {
        let mut out = format!(
            "# Execution Plan\n\n## Goal\n\n{}\n\n## Work Mode\n\n{}\n\n## Strategy\n\nLocal-first in mixed mode; strict provider boundaries otherwise. Workers produce proposals.\n\n## Provider Strategy\n\nLocal: analysis, implementation proposals, tests and documentation when capable.\nRemote: material reasoning gains, capability/context gaps and bounded escalation.\n\n## Tasks\n\n",
            self.goal,
            match self.work_mode {
                WorkMode::Local => "local",
                WorkMode::Mixed => "mixed",
                WorkMode::FullRemote => "full_remote",
            }
        );
        for t in &self.tasks {
            // Writing to String cannot fail.
            let _ = writeln!(
                out,
                "### {} — {}\n\nPreferred provider: {}\nDifficulty: {:?}\nRisk: {:?}\nDependencies: {}\n\n{}\n\nReason: {}\n\nContext: {:?}; requirements: {}\n\nExpected outputs: {}\n\nValidation: {:?}\n\nEscalation: at most {} local attempts; remote permitted by task: {} (work mode and budgets still apply).\n",
                t.id.0,
                t.title,
                t.preferred_provider.key(),
                t.difficulty,
                t.risk,
                if t.dependencies.is_empty() {
                    "None".into()
                } else {
                    t.dependencies
                        .iter()
                        .map(|d| d.0.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                },
                t.description,
                t.reason,
                t.context_strategy,
                t.context_requirements.join("; "),
                t.expected_outputs.join(", "),
                t.validation,
                t.escalation_policy.max_local_attempts,
                t.escalation_policy.allow_remote
            );
        }
        out.push_str("## Execution Waves\n\n");
        for (i, wave) in self.waves()?.iter().enumerate() {
            let _ = writeln!(out, "### Wave {}\n", i + 1);
            for id in wave {
                let _ = writeln!(out, "- {}", id.0);
            }
            out.push('\n');
        }
        out.push_str("## Expected Remote Calls\n\n");
        let remote: Vec<_> = self
            .tasks
            .iter()
            .filter(|t| t.preferred_provider == ProviderPreference::Remote)
            .collect();
        if remote.is_empty() {
            out.push_str("None initially.\n");
        }
        for t in remote {
            let _ = writeln!(out, "- {} — {}", t.id.0, t.title);
        }
        out.push_str("\nMixed-mode retries may add one remote call per eligible task. Actual calls and reported token usage appear in run.json. Estimates do not guarantee task correctness.\n");
        Ok(out)
    }
}
