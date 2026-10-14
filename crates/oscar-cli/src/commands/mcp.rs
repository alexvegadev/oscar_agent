use crate::error::CliError;
use clap::{Arg, ArgMatches, Command};
use oscar_core::{config::Config, error::OscarError};

pub(crate) fn command() -> Command {
    Command::new("mcp")
        .about("Inspect configured MCP servers and validate tool registration")
        .arg(
            Arg::new("config")
                .long("config")
                .default_value("settings.toml")
                .global(true),
        )
        .subcommand_required(true)
        .subcommand(
            Command::new("list")
                .about("List host configuration without connecting or reading credentials"),
        )
        .subcommand(Command::new("check").about(
            "Connect enabled servers and validate selected tool schemas; does not call tools",
        ))
}
pub(crate) fn handle(arguments: &ArgMatches) -> Result<(), CliError> {
    let config = Config::load_from_file(
        arguments
            .get_one::<String>("config")
            .map(String::as_str)
            .unwrap_or("settings.toml"),
    )?;
    match arguments.subcommand_name() {
        Some("list") => {
            println!("MCP enabled: {}", config.features.mcp_enabled);
            for (name, server) in &config.mcp.servers {
                println!(
                    "{name}: {:?}, enabled={}, selected_tools={}",
                    server.transport,
                    server.enabled,
                    server.tools.len()
                );
            }
            Ok(())
        }
        Some("check") => check(config),
        _ => Err(CliError::MissingCommand),
    }
}
#[cfg(feature = "mcp")]
fn check(config: Config) -> Result<(), CliError> {
    if !config.features.mcp_enabled {
        return Err(OscarError::Config(
            "set features.mcp_enabled = true before MCP discovery".into(),
        )
        .into());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|_| OscarError::Config("cannot initialize MCP runtime".into()))?;
    let registry = runtime.block_on(async {
        let cancellation = oscar_core::execution::CancellationToken::new();
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => { cancellation.cancel(); Err(OscarError::Cancelled) },
            result = oscar_core::mcp::register_servers(Default::default(), &config, cancellation.clone()) => result,
        }
    })?;
    for definition in registry.definitions() {
        println!("{}: {:?}", definition.name, definition.effect);
    }
    println!(
        "MCP registration verified: {} tools",
        registry.definitions().count()
    );
    Ok(())
}
#[cfg(not(feature = "mcp"))]
fn check(_: Config) -> Result<(), CliError> {
    Err(OscarError::Config("MCP discovery requires --features mcp".into()).into())
}
