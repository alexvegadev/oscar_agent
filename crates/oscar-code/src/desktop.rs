use crate::{
    session::{Role, SessionState},
    settings::Settings,
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    Disableable, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
};
use oscar_core::{
    config::{Capability, Config, WorkMode},
    error::OscarError,
    planning::ProviderPreference,
    providers::Providers,
};
use std::{path::PathBuf, sync::Arc};

pub struct Chat {
    session: SessionState,
    providers: Providers,
    config: Config,
    settings: Settings,
    settings_path: PathBuf,
    runtime: tokio::runtime::Handle,
    input: Entity<TextareaState>,
    _input_subscription: Subscription,
    turns: Entity<InputState>,
    timeout: Entity<InputState>,
    settings_open: bool,
    notice: String,
    scroll: ScrollHandle,
    task: Option<Task<()>>,
    saving: bool,
}

impl Chat {
    pub fn new(
        config: Config,
        settings: Settings,
        settings_path: PathBuf,
        runtime: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Self, OscarError> {
        let providers = Providers::from_config(&config)?;
        providers.get(settings.provider)?;
        if config.planning.local_only_data && settings.provider == ProviderPreference::Remote {
            return Err(OscarError::Config(
                "local-only data cannot use the desktop remote provider".into(),
            ));
        }
        let max_input_bytes = config.limits.max_input_bytes;
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask OSCAR for a coding proposal…")
                .auto_grow(3, 8)
        });
        let input_subscription = cx.subscribe_in(
            &input,
            window,
            move |this: &mut Self, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value();
                    if value.len() > max_input_bytes {
                        let mut end = max_input_bytes;
                        while !value.is_char_boundary(end) {
                            end -= 1;
                        }
                        input.update(cx, |input, cx| {
                            input.set_value(value[..end].to_owned(), window, cx)
                        });
                        this.notice = "Message clipped at the configured input byte limit".into();
                        cx.notify();
                    }
                }
            },
        );
        Ok(Self {
            session: SessionState::new(config.limits.clone(), settings.clone()),
            providers,
            input,
            _input_subscription: input_subscription,
            turns: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(settings.max_turns.to_string())
                    .validate(|value, _| value.len() <= 3)
            }),
            timeout: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(settings.timeout_ms.to_string())
                    .validate(|value, _| value.len() <= 7)
            }),
            config,
            settings,
            settings_path,
            runtime,
            settings_open: false,
            notice: String::new(),
            scroll: ScrollHandle::new(),
            task: None,
            saving: false,
        })
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if self.session.update_settings(self.settings.clone()).is_err() {
            return;
        }
        let configured = &self.config.providers[self.settings.provider.key()];
        if (self.config.planning.local_only_data
            && self.settings.provider == ProviderPreference::Remote)
            || !configured.capabilities.contains(&Capability::CodeAnalysis)
            || !configured
                .capabilities
                .contains(&Capability::CodeGeneration)
        {
            self.notice =
                "Selected provider is blocked by local-only policy or lacks coding capabilities"
                    .into();
            cx.notify();
            return;
        }
        let prompt = self.input.read(cx).value().to_string();
        let provider = match self.providers.get(self.settings.provider) {
            Ok(provider) => provider,
            Err(_) => {
                self.notice = "Selected provider is unavailable".into();
                cx.notify();
                return;
            }
        };
        let run = match self.session.begin(&prompt) {
            Ok(run) => run,
            Err(error) => {
                self.notice = error.to_string();
                cx.notify();
                return;
            }
        };
        let id = run.id;
        let configured = &self.config.providers[self.settings.provider.key()];
        let context_tokens = run
            .input_bytes()
            .saturating_add(self.config.limits.max_output_tokens as usize)
            .saturating_add(256);
        let cost_allowed = if self.settings.provider == ProviderPreference::Remote {
            self.config.routing.max_remote_call_cost.is_none_or(|cap| {
                match (
                    configured.cost.input_per_million,
                    configured.cost.output_per_million,
                ) {
                    (Some(input), Some(output)) => {
                        (run.input_bytes() as f64 * input
                            + self.config.limits.max_output_tokens as f64 * output)
                            / 1_000_000.
                            <= cap
                    }
                    _ => false,
                }
            })
        } else {
            true
        };
        if context_tokens > configured.context_window as usize || !cost_allowed {
            self.session.complete(
                id,
                Err(OscarError::Limit(
                    "provider context or remote cost bound exceeded".into(),
                )),
            );
            self.notice = "Provider context or remote cost bound exceeded".into();
            cx.notify();
            return;
        }
        self.notice.clear();
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.session.running(id);
        let (sender, mut receiver) = tokio::sync::watch::channel(String::new());
        let mut handle = self.runtime.spawn(async move {
            let mut text = String::new();
            run.execute_stream(provider, &mut |delta| {
                text.push_str(delta);
                sender.send_replace(text.clone());
                Ok(())
            })
            .await
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = loop {
                tokio::select! {
                    biased;
                    result = &mut handle => break result.unwrap_or_else(|_| {
                        Err(OscarError::Unavailable("inference worker stopped".into()))
                    }),
                    changed = receiver.changed() => {
                        if changed.is_err() {
                            break handle.await.unwrap_or_else(|_| Err(OscarError::Unavailable("inference worker stopped".into())));
                        }
                        let text = receiver.borrow_and_update().clone();
                        if this.update(cx, |this, cx| {
                            this.session.preview(id, text);
                            this.scroll.scroll_to_bottom();
                            cx.notify();
                        }).is_err() {
                            handle.abort();
                            return;
                        }
                    }
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.session.complete(id, result);
                this.scroll.scroll_to_bottom();
                cx.notify();
            });
        }));
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.session.state.active() {
            return;
        }
        let Ok(max_turns) = self.turns.read(cx).value().parse() else {
            self.notice = "Turn limit must be an integer".into();
            cx.notify();
            return;
        };
        let Ok(timeout_ms) = self.timeout.read(cx).value().parse() else {
            self.notice = "Timeout must be an integer in milliseconds".into();
            cx.notify();
            return;
        };
        let settings = Settings {
            max_turns,
            timeout_ms,
            ..self.settings.clone()
        };
        if let Err(error) = settings.validate(self.config.work_mode) {
            self.notice = error.to_string();
            cx.notify();
            return;
        }
        let path = self.settings_path.clone();
        let mode = self.config.work_mode;
        let to_save = settings.clone();
        let writer = self
            .runtime
            .spawn_blocking(move || to_save.save(&path, mode));
        let handle = self.runtime.spawn(async move {
            tokio::time::timeout(std::time::Duration::from_secs(5), writer).await
        });
        self.saving = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = handle.await;
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(Ok(Ok(Ok(())))) => {
                        this.settings = settings.clone();
                        // No inference can start while saving.
                        if this.session.update_settings(settings).is_ok() {
                            this.notice = "Preferences saved".into();
                        }
                    }
                    Ok(Err(_)) => {
                        // An uncancellable write is still pending. Disable retries
                        // to prevent accumulation of blocking jobs or late overwrites.
                        this.saving = true;
                        this.notice = "Preferences save timed out and may still finish. Restart before further changes.".into();
                    }
                    _ => this.notice = "Preferences save failed or timed out; filesystem writes cannot be cancelled and may still finish".into(),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn message(index: usize, role: Role, content: &str, dark: bool) -> AnyElement {
        let title = match role {
            Role::User => "You",
            Role::Model => "OSCAR",
            Role::Tool => "Tool",
            Role::System => "System",
        };
        let body = if role == Role::Model {
            // Untrusted Markdown must not fetch external images or open URLs.
            gpui_kit::base::TextView::markdown(("body", index), content.to_owned())
                .selectable(true)
                .scrollable(false)
                .on_link_click(|_, _, _, _| {})
                .image_source(|_| {
                    ImageSource::Image(Arc::new(Image::from_bytes(
                        ImageFormat::Svg,
                        b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>".to_vec(),
                    )))
                })
                .into_any_element()
        } else {
            div()
                .whitespace_normal()
                .child(content.to_owned())
                .into_any_element()
        };
        div()
            .id(("message", index))
            .w_full()
            .p_4()
            .rounded_lg()
            .bg(rgb(if dark {
                if role == Role::User {
                    0x242a35
                } else {
                    0x191d25
                }
            } else if role == Role::User {
                0xe8edf5
            } else {
                0xf8fafc
            }))
            .child(
                div()
                    .text_sm()
                    .mb_2()
                    .text_color(rgb(0x7f9fbc))
                    .child(title),
            )
            .child(body)
            .into_any_element()
    }
}

impl Render for Chat {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.session.state.active();
        let dark = self.settings.dark;
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .p_4()
            .border_b_1()
            .border_color(rgb(0x384355))
            .child(div().child("OSCAR Code").child(
                div().text_sm().text_color(rgb(0x7f9fbc)).child(format!(
                    "{:?} · Run {} · Turns {}/{}",
                    self.session.state,
                    self.session.run_id(),
                    self.session.turns,
                    self.settings.max_turns
                )),
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("reset")
                            .label("Reset")
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session.reset();
                                this.notice.clear();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("settings")
                            .label("Settings")
                            .disabled(active || self.saving)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings_open = !this.settings_open;
                                cx.notify();
                            })),
                    ),
            );
        if self.settings_open {
            header = header.flex_col().items_start().gap_3()
                .child(div().text_sm().child(format!("Provider: {:?} · Model: {}", self.settings.provider, self.config.providers.get(self.settings.provider.key()).map(|p| p.model.as_str()).unwrap_or("Unavailable"))))
                .child(div().text_sm().child("Models and API key environment references: settings.toml / --config. Credentials are never saved here."))
                .child(div().flex().gap_2().items_center()
                    .child("Turn limit").child(Input::new(&self.turns).w(px(90.)))
                    .child("Timeout (ms)").child(Input::new(&self.timeout).w(px(140.)))
                    .child(Button::new("provider").label(if self.settings.provider == ProviderPreference::Local { "Use remote (share chat)" } else { "Use local" }).disabled(active || self.saving || self.config.planning.local_only_data || self.config.work_mode != WorkMode::Mixed || self.providers.remote.is_none()).on_click(cx.listener(|this, _, _, cx| {
                        this.settings.provider = if this.settings.provider == ProviderPreference::Local { ProviderPreference::Remote } else { ProviderPreference::Local }; cx.notify();
                    })))
                    .child(Button::new("theme").label("Dark / Light").disabled(self.saving).on_click(cx.listener(|this, _, window, cx| {
                        this.settings.dark = !this.settings.dark; Theme::change(if this.settings.dark { ThemeMode::Dark } else { ThemeMode::Light }, Some(window), cx); cx.notify();
                    })))
                    .child(Button::new("save").label(if self.saving { "Saving…" } else { "Save preferences" }).disabled(active || self.saving).on_click(cx.listener(|this, _, _, cx| this.save_settings(cx)))));
        }
        div().size_full().flex().flex_col().bg(rgb(if dark { 0x10141b } else { 0xffffff })).text_color(rgb(if dark { 0xe2e8f0 } else { 0x1b2430 }))
            .child(header)
            .child(div().id("history").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.scroll).p_6()
                .child(div().w_full().max_w(px(900.)).mx_auto().flex().flex_col().gap_4()
                    .when(self.session.messages.is_empty(), |view| view.child(div().p_6().child("What would you like to build?").child(div().text_sm().mt_2().child("Ask for a coding proposal. This session keeps conversation in memory."))))
                    .children(self.session.messages.iter().enumerate().map(|(i, m)| Self::message(i, m.role, &m.content, dark)))
                    .when(!self.session.preview.is_empty(), |view| view.child(Self::message(self.session.messages.len(), Role::Model, &self.session.preview, dark)))))
            .child(div().p_4().border_t_1().border_color(rgb(0x384355)).flex().flex_col().gap_2()
                .child(div().text_sm().child(self.notice.clone()))
                .child(Textarea::new(&self.input).w_full().disabled(active || self.saving).aria_label("Message OSCAR"))
                .child(div().flex().justify_between().items_center()
                    .child(div().text_sm().text_color(rgb(0x7f9fbc)).child("Proposals only · Enter inserts a new line · Reset preserves turn budget"))
                    .child(if active {
                        Button::new("stop").label("Stop Generation").on_click(cx.listener(|this, _, _, cx| { this.session.stop(); cx.notify(); }))
                    } else {
                        Button::new("submit").primary().label("Send").disabled(self.saving).on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))
                    })))
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::Chat;
    use crate::{session::RunState, settings::Settings};
    use gpui::{AppContext, Bounds, TestAppContext, WindowBounds, WindowOptions, px, size};
    use gpui_kit::test::TestWindowExt;
    use oscar_core::config::Config;

    #[gpui::test]
    fn actual_buttons_submit_stop_reset_and_change_theme(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let handle = runtime.handle().clone();
        let (window, chat) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1200.), px(900.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        Chat::new(
                            Config::parse(include_str!("../../../settings.toml")).unwrap(),
                            Settings::default(),
                            "unused-ui-test.toml".into(),
                            handle,
                            window,
                            cx,
                        )
                        .unwrap()
                    })
                },
            )
            .unwrap()
        });
        cx.update_window(window, |_, window, cx| {
            chat.update(cx, |chat, cx| {
                chat.input.update(cx, |input, cx| {
                    input.set_value("Write a parser", window, cx)
                })
            });
            window.render_frame(cx);
            window.click("submit", cx);
            assert_eq!(chat.read(cx).session.state, RunState::Running);
            window.render_frame(cx);
            window.click("stop", cx);
            assert_eq!(chat.read(cx).session.state, RunState::Cancelled);
            window.render_frame(cx);
            window.click("reset", cx);
            assert!(chat.read(cx).session.messages.is_empty());
            assert_eq!(chat.read(cx).session.turns, 1);
            window.render_frame(cx);
            window.click("settings", cx);
            assert!(chat.read(cx).settings_open);
            window.render_frame(cx);
            window.click("theme", cx);
            assert!(!chat.read(cx).settings.dark);
        })
        .unwrap();
        runtime.shutdown_timeout(std::time::Duration::from_secs(1));
    }
}
