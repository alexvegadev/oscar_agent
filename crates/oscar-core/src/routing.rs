use crate::{
    config::{Config, WorkMode},
    error::OscarError,
    planning::{Difficulty, ProviderPreference as P, RiskLevel, Task, TaskKind},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub provider: P,
    pub reason: &'static str,
}

fn available(config: &Config, task: &Task, choice: P, input_bytes: usize) -> bool {
    if choice == P::Remote
        && (task.local_only_data
            || !task.escalation_policy.allow_remote && config.work_mode == WorkMode::Mixed)
    {
        return false;
    }
    config.providers.get(choice.key()).is_some_and(|p| p.enabled
        && task.required_capabilities.iter().all(|c| p.capabilities.contains(c))
        // One token per UTF-8 byte is a conservative estimate, with envelope reserve.
        && input_bytes.saturating_add(config.limits.max_output_tokens as usize).saturating_add(256) <= p.context_window as usize)
}

pub fn select(
    config: &Config,
    task: &Task,
    local_failures: usize,
    input_bytes: usize,
) -> Result<Decision, OscarError> {
    select_with_context_sizes(config, task, local_failures, input_bytes, input_bytes)
}

/// Evaluate each provider against the context it would actually receive.
pub(crate) fn select_with_context_sizes(
    config: &Config,
    task: &Task,
    local_failures: usize,
    local_bytes: usize,
    remote_bytes: usize,
) -> Result<Decision, OscarError> {
    let local = available(config, task, P::Local, local_bytes);
    let remote = available(config, task, P::Remote, remote_bytes);
    let decision = match config.work_mode {
        WorkMode::Local if local => Decision {
            provider: P::Local,
            reason: "strict local mode",
        },
        WorkMode::FullRemote if remote => Decision {
            provider: P::Remote,
            reason: "strict full_remote mode",
        },
        WorkMode::Local | WorkMode::FullRemote => {
            return Err(OscarError::Unavailable(
                "strict mode provider cannot satisfy capability, context or data boundary".into(),
            ));
        }
        WorkMode::Mixed => {
            let exhausted = local_failures
                >= task
                    .escalation_policy
                    .max_local_attempts
                    .min(config.routing.max_local_attempts);
            let reasoning = matches!(task.kind, TaskKind::Reason | TaskKind::Review);
            let quality_gain = reasoning
                && (matches!(task.risk, RiskLevel::High | RiskLevel::Critical)
                    || matches!(task.difficulty, Difficulty::High | Difficulty::Critical));
            if remote
                && (!local
                    || exhausted
                    || quality_gain
                    || (!config.routing.local_first && task.preferred_provider == P::Remote))
            {
                Decision {
                    provider: P::Remote,
                    reason: if exhausted {
                        "local attempts exhausted"
                    } else if !local {
                        "local capability or context gap"
                    } else {
                        "reasoning quality gain justifies remote use"
                    },
                }
            } else if local && !exhausted {
                Decision {
                    provider: P::Local,
                    reason: "local-first; capability and context fit",
                }
            } else {
                return Err(OscarError::Unavailable("local attempts exhausted or capability/context unavailable; remote is disabled or forbidden".into()));
            }
        }
    };
    if decision.provider == P::Remote {
        if config.limits.max_remote_calls == 0 {
            if config.work_mode == WorkMode::Mixed
                && local
                && local_failures
                    < task
                        .escalation_policy
                        .max_local_attempts
                        .min(config.routing.max_local_attempts)
            {
                return Ok(Decision {
                    provider: P::Local,
                    reason: "remote budget disabled; local remains capable",
                });
            }
            return Err(OscarError::Limit("remote calls disabled by budget".into()));
        }
        if let Some(cap) = config.routing.max_remote_call_cost {
            let p = &config.providers["remote"];
            let (Some(input), Some(output)) = (p.cost.input_per_million, p.cost.output_per_million)
            else {
                return Err(OscarError::Unavailable(
                    "remote cost ceiling requires input/output price metadata".into(),
                ));
            };
            let estimate = (remote_bytes as f64 * input
                + config.limits.max_output_tokens as f64 * output)
                / 1_000_000.0;
            if estimate > cap {
                if config.work_mode == WorkMode::Mixed
                    && local
                    && local_failures < task.escalation_policy.max_local_attempts
                {
                    return Ok(Decision {
                        provider: P::Local,
                        reason: "remote cost ceiling favors local",
                    });
                }
                return Err(OscarError::Limit(
                    "estimated remote call cost exceeds ceiling".into(),
                ));
            }
        }
    }
    Ok(decision)
}
