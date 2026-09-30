use crate::error::CliError;
use clap::{Arg, ArgMatches, Command};
use oscar_core::{
    config::Config,
    error::OscarError,
    execution::{self, CancellationToken, Outcome, Validators},
    planning::{Plan, plan_request},
    providers::Providers,
};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn common(name: &'static str) -> Command {
    Command::new(name)
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
            Arg::new("plan")
                .long("plan")
                .help("Load a canonical JSON plan instead of analyzing a request"),
        )
}
pub(super) fn plan_command() -> Command {
    common("plan").about("Write canonical .plan/plan.json and PLAN.md without inference")
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
fn prepare(args: &ArgMatches) -> Result<(Config, Plan, PathBuf), CliError> {
    let config = Config::load_from_file(argument(args, "config")?)?;
    let plan = if let Some(path) = args.get_one::<String>("plan") {
        let mut text = String::new();
        fs::File::open(path)
            .map_err(|_| OscarError::Io("cannot open plan".into()))?
            .take(2_097_153)
            .read_to_string(&mut text)
            .map_err(|_| OscarError::Io("cannot read UTF-8 plan".into()))?;
        Plan::from_json(&text, &config)?
    } else {
        plan_request(argument(args, "request")?, &config)?
    };
    let path = PathBuf::from(argument(args, "out")?);
    Ok((config, plan, path))
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
    fs::create_dir(path).map_err(|_| {
        OscarError::Io("--out must name a new directory under an existing parent".into())
    })?;
    fs::create_dir(path.join(".plan"))
        .map_err(|_| OscarError::Io("cannot create plan directory".into()))?;
    write_new(&path.join(".plan/plan.json"), &json)?;
    write_new(&path.join("PLAN.md"), &markdown)
}
pub(super) fn plan_handle(args: &ArgMatches) -> Result<(), CliError> {
    let (_, plan, path) = prepare(args)?;
    write_plan(&path, &plan)?;
    println!(
        "Wrote PLAN.md and .plan/plan.json ({} tasks; no inference).",
        plan.tasks.len()
    );
    Ok(())
}
pub(super) fn run_handle(args: &ArgMatches) -> Result<(), CliError> {
    let (config, plan, path) = prepare(args)?;
    let providers = Providers::from_config(&config)?;
    write_plan(&path, &plan)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| OscarError::Config("cannot create async runtime".into()))?;
    let report = runtime.block_on(async {
        let cancel = CancellationToken::new();
        let signal = cancel.clone();
        let listener = tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                signal.cancel();
            }
        });
        let result = execution::execute(plan, config, providers, Validators::new(), cancel).await;
        listener.abort();
        result
    })?;
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
