use gpui::{AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_kit::component::{Theme, ThemeMode};
use oscar_code::{desktop::Chat, settings::Settings};
use oscar_core::config::Config;
use std::path::PathBuf;

fn main() {
    if let Err(error) = start() {
        eprintln!("oscar-code: {error}");
        std::process::exit(1);
    }
}

fn start() -> Result<(), Box<dyn std::error::Error>> {
    let mut config_path = "settings.toml".to_owned();
    let mut settings_path = PathBuf::from("oscar-code.toml");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config_path = args.next().ok_or("--config requires a path")?,
            "--ui-settings" => {
                settings_path = args.next().ok_or("--ui-settings requires a path")?.into()
            }
            "--help" | "-h" => {
                println!("oscar-code [--config settings.toml] [--ui-settings oscar-code.toml]");
                return Ok(());
            }
            _ => return Err("unknown argument; use --help".into()),
        }
    }
    // Prevent preferences from accidentally overwriting provider configuration.
    let absolute = |path: &std::path::Path| -> std::io::Result<PathBuf> {
        if path.exists() {
            path.canonicalize()
        } else {
            Ok(path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."))
                .canonicalize()?
                .join(
                    path.file_name()
                        .ok_or_else(|| std::io::Error::other("missing filename"))?,
                ))
        }
    };
    if absolute(std::path::Path::new(&config_path))? == absolute(&settings_path)? {
        return Err("provider config and desktop preferences must use different files".into());
    }
    let config = Config::load_from_file(&config_path)?;
    let settings = Settings::load(&settings_path, config.work_mode)?;
    // Validate adapters before entering the platform event loop.
    oscar_core::providers::Providers::from_config(&config)?.get(settings.provider)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()?;
    let runtime_handle = runtime.handle().clone();
    let (registry, tool_policy) = runtime.block_on(oscar_code::runtime::configured_tools(
        &config,
        Default::default(),
    ))?;
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(
                if settings.dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                },
                None,
                cx,
            );
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let result = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1100.), px(800.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        // The same adapter configuration was validated above; return a controlled
                        // startup failure rather than opening a broken session if it changes.
                        match Chat::new(config, settings, settings_path, runtime_handle, window, cx)
                        {
                            Ok(mut chat) => {
                                if chat.set_tools(registry, tool_policy).is_err() {
                                    eprintln!("oscar-code: cannot install configured tools");
                                    std::process::exit(1);
                                }
                                chat
                            }
                            Err(_) => {
                                eprintln!("oscar-code: cannot initialize desktop session");
                                std::process::exit(1);
                            }
                        }
                    })
                },
            );
            if result.is_err() {
                eprintln!("oscar-code: cannot open the main window");
                cx.quit();
            }
            cx.activate(true);
        });
    runtime.shutdown_timeout(std::time::Duration::from_secs(1));
    Ok(())
}
