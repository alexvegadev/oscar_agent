use clap::Command;

use crate::{commands, error::CliError};

pub(crate) fn command() -> Command {
    commands::register(
        Command::new("oscar")
            .version(env!("CARGO_PKG_VERSION"))
            .about("OSCAR command-line interface")
            .subcommand_required(true)
            .arg_required_else_help(true),
    )
}

pub(crate) fn run() -> Result<(), CliError> {
    let matches = command().get_matches();
    commands::dispatch(&matches)
}

#[cfg(test)]
mod tests {
    use super::command;

    #[test]
    fn command_tree_is_valid() {
        command().debug_assert();
    }

    #[test]
    fn test_command_accepts_a_name() {
        let matches = command()
            .try_get_matches_from(["oscar", "test", "--name", "smoke"])
            .expect("valid CLI arguments should parse");

        assert_eq!(matches.subcommand_name(), Some("test"));
    }

    #[test]
    fn test_command_requires_a_name() {
        let result = command().try_get_matches_from(["oscar", "test"]);

        assert!(result.is_err());
    }
}
