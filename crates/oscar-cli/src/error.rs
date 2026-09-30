use std::{error::Error, fmt};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CliError {
    MissingCommand,
    MissingArgument {
        command: &'static str,
        argument: &'static str,
    },
    UnknownCommand(String),
    Orchestration(oscar_core::error::OscarError),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCommand => formatter.write_str("a command is required"),
            Self::MissingArgument { command, argument } => {
                write!(
                    formatter,
                    "command '{command}' requires argument '{argument}'"
                )
            }
            Self::UnknownCommand(command) => write!(formatter, "unknown command '{command}'"),
            Self::Orchestration(error) => error.fmt(formatter),
        }
    }
}

impl Error for CliError {}

impl From<oscar_core::error::OscarError> for CliError {
    fn from(error: oscar_core::error::OscarError) -> Self {
        Self::Orchestration(error)
    }
}
