mod orchestration;
mod test;

use clap::{ArgMatches, Command};

use crate::error::CliError;

type Handler = fn(&ArgMatches) -> Result<(), CliError>;

struct CommandSpec {
    name: &'static str,
    build: fn() -> Command,
    handle: Handler,
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: test::NAME,
        build: test::command,
        handle: test::handle,
    },
    CommandSpec {
        name: "plan",
        build: orchestration::plan_command,
        handle: orchestration::plan_handle,
    },
    CommandSpec {
        name: "run",
        build: orchestration::run_command,
        handle: orchestration::run_handle,
    },
];

pub(crate) fn register(root: Command) -> Command {
    COMMANDS
        .iter()
        .fold(root, |command, spec| command.subcommand((spec.build)()))
}

pub(crate) fn dispatch(matches: &ArgMatches) -> Result<(), CliError> {
    let (name, arguments) = matches.subcommand().ok_or(CliError::MissingCommand)?;

    let spec = COMMANDS
        .iter()
        .find(|spec| spec.name == name)
        .ok_or_else(|| CliError::UnknownCommand(name.to_owned()))?;

    (spec.handle)(arguments)
}
