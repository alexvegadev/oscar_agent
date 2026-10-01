use crate::error::CliError;
use clap::{Arg, ArgAction, ArgMatches, Command};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use oscar_core::{
    config::{Config, PlannerMode},
    error::OscarError,
    execution::{self, CancellationToken, Outcome, Validators},
    planning::{Plan, inference, plan_request},
    providers::Providers,
};
use std::{
    fs::{self, OpenOptions},
    future::Future,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

fn status(quiet: bool, message: &str) {
    if !quiet {
        eprintln!("[oscar] {message}");
    }
}

struct Spinner(ProgressBar);

impl Spinner {
    fn new(stage: &str, target: ProgressDrawTarget) -> Result<Self, CliError> {
        let style =
            ProgressStyle::with_template("{spinner:.cyan} [oscar] {wide_msg} [{elapsed_precise}]")
                .map_err(|_| OscarError::Config("invalid progress style".into()))?
                .tick_strings(&["|", "/", "-", "\\", " "]);
        let bar = ProgressBar::with_draw_target(None, target).with_style(style);
        bar.set_message(stage.to_owned());
        Ok(Self(bar))
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        // Clear the transient display even when the operation future is dropped.
        self.0.finish_and_clear();
    }
}

// Poll the operation and spinner together, without a background ticking thread.
// Existing inference deadlines/cancellation still govern the operation.
async fn with_progress<T>(
    quiet: bool,
    stage: &str,
    operation: impl Future<Output = Result<T, CliError>>,
) -> Result<T, CliError> {
    if quiet {
        return operation.await;
    }
    let spinner = Spinner::new(stage, ProgressDrawTarget::stderr())?;
    let animated = !spinner.0.is_hidden();
    if !animated {
        status(false, stage);
    }
    let started = tokio::time::Instant::now();
    let period = Duration::from_millis(100);
    let mut ticks = tokio::time::interval_at(started + period, period);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tokio::pin!(operation);
    loop {
        tokio::select! {
            biased;
            result = &mut operation => {
                drop(spinner);
                status(false, &format!("{stage}: {} ({}s elapsed)",
                    if result.is_ok() { "finished" } else { "failed" },
                    started.elapsed().as_secs()));
                return result;
            }
            _ = ticks.tick(), if animated => {
                spinner.0.tick();
            }
        }
    }
}

fn common(name: &'static str) -> Command {
    Command::new(name)
        .arg(
            Arg::new("quiet")
                .long("quiet")
                .short('q')
                .action(ArgAction::SetTrue)
                .help("Suppress progress messages on stderr; keep summaries and errors"),
        )
        .arg(
            Arg::new("request")
                .value_name("REQUEST")
                .required_unless_present("plan")
                .conflicts_with("plan"),
        )
        .arg(
            Arg::new("config")
                .long("config")
                .default_value("example_config.toml"),
        )
        .arg(
            Arg::new("out")
                .long("out")
                .required(true)
                .help("New output directory; existing directories are never overwritten"),
        )
        .arg(
            Arg::new("planner")
                .long("planner")
                .value_parser(["heuristic", "inference"])
                .conflicts_with("plan")
                .help("Override planning.mode from TOML"),
        )
        .arg(
            Arg::new("plan")
                .long("plan")
                .help("Load a canonical JSON plan instead of analyzing a request"),
        )
}
pub(super) fn plan_command() -> Command {
    common("plan")
        .about("Create a validated plan with the configured heuristic or inference planner")
}
pub(super) fn run_command() -> Command {
    common("run").about("Plan and execute proposal workers; persist artifacts and inference report")
}
fn argument<'a>(args: &'a ArgMatches, key: &'static str) -> Result<&'a str, CliError> {
    args.get_one::<String>(key)
        .map(String::as_str)
        .ok_or(CliError::MissingArgument {
            command: "plan/run",
            argument: key,
        })
}
async fn prepare(
    args: &ArgMatches,
    config: &mut Config,
    path: &Path,
    providers: &Providers,
    cancel: CancellationToken,
    run: bool,
) -> Result<Plan, CliError> {
    let plan = if let Some(path) = args.get_one::<String>("plan") {
        let mut text = String::new();
        fs::File::open(path)
            .map_err(|_| OscarError::Io("cannot open plan".into()))?
            .take(2_097_153)
            .read_to_string(&mut text)
            .map_err(|_| OscarError::Io("cannot read UTF-8 plan".into()))?;
        let mut plan = Plan::from_json(&text, config)?;
        if config.planning.local_only_data {
            for task in &mut plan.tasks {
                task.local_only_data = true;
                task.escalation_policy.allow_remote = false;
            }
        }
        plan
    } else if config.planning.mode == PlannerMode::Inference {
        let result =
            inference::generate(argument(args, "request")?, config, providers, cancel).await;
        let report = serde_json::to_string_pretty(&result.report)
            .map_err(|_| OscarError::Io("cannot serialize planning report".into()))?;
        write_new(&path.join("planning.json"), &report)?;
        let plan = result.plan?;
        if run {
            *config = result.report.remaining_config(config)?;
        }
        plan
    } else {
        plan_request(argument(args, "request")?, config)?
    };
    Ok(plan)
}
fn write_new(path: &Path, text: &str) -> Result<(), OscarError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| {
            OscarError::Io("cannot create output file (possibly already exists)".into())
        })?;
    file.write_all(text.as_bytes())
        .map_err(|_| OscarError::Io("cannot write output file".into()))
}
fn write_plan(path: &Path, plan: &Plan) -> Result<(), OscarError> {
    let json = plan.to_json()?;
    let markdown = plan.render_markdown()?;
    fs::create_dir(path.join(".plan"))
        .map_err(|_| OscarError::Io("cannot create plan directory".into()))?;
    write_new(&path.join(".plan/plan.json"), &json)?;
    write_new(&path.join("PLAN.md"), &markdown)
}
pub(super) fn plan_handle(args: &ArgMatches) -> Result<(), CliError> {
    handle(args, false)
}
pub(super) fn run_handle(args: &ArgMatches) -> Result<(), CliError> {
    handle(args, true)
}
fn handle(args: &ArgMatches, run: bool) -> Result<(), CliError> {
    let quiet = args.get_flag("quiet");
    status(quiet, "Loading configuration");
    let mut config = Config::load_from_file(argument(args, "config")?)?;
    if let Some(mode) = args.get_one::<String>("planner") {
        config.planning.mode = if mode == "inference" {
            PlannerMode::Inference
        } else {
            PlannerMode::Heuristic
        };
    }
    let path = PathBuf::from(argument(args, "out")?);
    // Reserve the output directory before any potentially billable call.
    fs::create_dir(&path).map_err(|_| {
        OscarError::Io("--out must name a new directory under an existing parent".into())
    })?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| OscarError::Config("cannot create async runtime".into()))?;
    runtime.block_on(async {
        let cancel = CancellationToken::new();
        let signal = cancel.clone();
        let listener = tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                signal.cancel();
            }
        });
        let result = async {
            let uses_inference = args.get_one::<String>("plan").is_none()
                && config.planning.mode == PlannerMode::Inference;
            let providers = if run || uses_inference {
                Providers::from_config(&config)?
            } else {
                Providers::default()
            };
            let stage = if args.get_one::<String>("plan").is_some() {
                "Loading and validating saved plan"
            } else if uses_inference {
                "Generating and validating plan with inference"
            } else {
                "Generating and validating heuristic plan"
            };
            let plan = with_progress(
                quiet,
                stage,
                prepare(args, &mut config, &path, &providers, cancel.clone(), run),
            )
            .await?;
            status(quiet, "Saving plan files");
            write_plan(&path, &plan)?;
            if !run {
                println!(
                    "Wrote PLAN.md and .plan/plan.json ({} tasks; {} planner).",
                    plan.tasks.len(),
                    if uses_inference {
                        "inference; see planning.json"
                    } else {
                        "saved/heuristic; no inference"
                    }
                );
                return Ok(());
            }
            let stage = format!(
                "Executing proposal workers ({} planned tasks)",
                plan.tasks.len()
            );
            let report = with_progress(quiet, &stage, async {
                Ok(execution::execute(plan, config, providers, Validators::new(), cancel).await?)
            })
            .await?;
            status(quiet, "Saving run report and artifacts");
            write_report(&path, report)
        }
        .await;
        listener.abort();
        result
    })
}
fn write_report(path: &Path, report: execution::RunReport) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(&report)
        .map_err(|_| OscarError::Io("cannot serialize run report".into()))?;
    write_new(&path.join("run.json"), &json)?;
    fs::create_dir(path.join("artifacts"))
        .map_err(|_| OscarError::Io("cannot create artifacts directory".into()))?;
    for (id, artifact) in &report.artifacts {
        write_new(
            &path.join("artifacts").join(format!("{}.md", id.0)),
            &artifact.content,
        )?;
    }
    let remote = report
        .calls
        .iter()
        .filter(|c| c.provider == oscar_core::planning::ProviderPreference::Remote)
        .count();
    let simulated = report.calls.iter().filter(|c| c.simulation).count();
    println!(
        "Outcome: {:?}; calls: {}; remote selections: {}; simulated calls: {}. See run.json for usage and validation evidence.",
        report.outcome,
        report.calls.len(),
        remote,
        simulated
    );
    if report.outcome != Outcome::Completed {
        return Err(OscarError::Validation(
            report
                .error
                .unwrap_or_else(|| "run did not complete".into()),
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    use indicatif::InMemoryTerm;

    #[test]
    fn spinner_redraws_one_line_and_clears_on_drop() {
        let terminal = InMemoryTerm::new(6, 100);
        let spinner = Spinner::new(
            "Generating plan",
            ProgressDrawTarget::term_like(Box::new(terminal.clone())),
        )
        .unwrap();
        assert!(!spinner.0.is_hidden());
        for _ in 0..20 {
            spinner.0.tick();
            spinner.0.force_draw();
            let screen = terminal.contents();
            assert_eq!(
                screen
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .count(),
                1
            );
            assert!(screen.contains("Generating plan") && screen.contains("00:00"));
        }
        drop(spinner);
        assert!(terminal.contents().trim().is_empty());
    }

    #[tokio::test]
    async fn quiet_progress_preserves_operation_errors() {
        let result = with_progress(true, "Hidden stage", async {
            Err::<(), _>(OscarError::Cancelled.into())
        })
        .await;
        assert!(result.is_err());
    }
}
