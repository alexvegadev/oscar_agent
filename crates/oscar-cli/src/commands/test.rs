use clap::{Arg, ArgMatches, Command};

use crate::error::CliError;

pub(super) const NAME: &str = "test";
const NAME_ARGUMENT: &str = "name";

pub(super) fn command() -> Command {
    Command::new(NAME).about("Execute a named test").arg(
        Arg::new(NAME_ARGUMENT)
            .long(NAME_ARGUMENT)
            .value_name("TEST_NAME")
            .help("The name of the test to execute")
            .required(true),
    )
}

pub(super) fn handle(matches: &ArgMatches) -> Result<(), CliError> {
    let name = matches
        .get_one::<String>(NAME_ARGUMENT)
        .ok_or(CliError::MissingArgument {
            command: NAME,
            argument: NAME_ARGUMENT,
        })?;

    println!("Running the test subcommand with --name: {name}");
    Ok(())
}
