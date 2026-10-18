//! Presentation only; Chat retains session, editor, dock and async ownership.
use super::*;
use gpui_kit::component::StyledExt as _;

gpui::actions!(oscar_chat, [ToggleEvents, OpenSettings]);

fn pill(label: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .text_xs()
        .child(label.into())
}

fn budget_row(label: &'static str, value: String, cx: &App) -> Div {
    div()
        .flex()
        .justify_between()
        .gap_2()
        .child(div().text_color(cx.theme().muted_foreground).child(label))
        .child(value)
}

impl Chat {
    pub(super) fn render_workspace(&self, cx: &mut Context<Self>) -> AnyElement {
        let ratio =
            (self.session.turns as f32 / self.settings.max_turns.max(1) as f32).clamp(0., 1.);
        div()
            .id("workspace")
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .gap_5()
            .p_5()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .h_8()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().size_2().rounded_full().bg(cx.theme().primary))
                    .child(div().text_lg().font_bold().child("OSCAR Code")),
            )
            .child(
                Button::new("reset")
                    .label("New conversation")
                    .w_full()
                    .h_10()
                    .rounded_md()
                    .disabled(self.saving)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.reset();
                        this.notice.clear();
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("WORKSPACE"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().font_semibold().child("Current session"))
                            .child(pill("In-Memory", cx)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Conversation stays in memory"),
                    )
                    .child(
                        div()
                            .mt_2()
                            .flex()
                            .justify_between()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Turns used")
                            .child(
                                div()
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .child(format!(
                                        "{} / {}",
                                        self.session.turns, self.settings.max_turns
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .h_1()
                            .w_full()
                            .rounded_full()
                            .bg(cx.theme().border)
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(ratio))
                                    .rounded_full()
                                    .bg(cx.theme().primary),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("EXECUTION BUDGET"),
                    )
                    .child(
                        div()
                            .p_4()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .rounded_lg()
                            .bg(cx.theme().background)
                            .border_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(budget_row(
                                "Tokens",
                                format!("— / {} max", self.config.limits.max_output_tokens),
                                cx,
                            ))
                            .child(budget_row(
                                "Time / call",
                                format!(
                                    "≤ {}s",
                                    self.settings
                                        .timeout_ms
                                        .min(self.config.limits.call_timeout_ms)
                                        .min(self.config.limits.run_timeout_ms)
                                        / 1000
                                ),
                                cx,
                            ))
                            .child(budget_row("Remote API", "Not reported".into(), cx)),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        Button::new("events")
                            .ghost()
                            .w_full()
                            .h_10()
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child("Run events / Trace")
                                    .child(pill("Ctrl+E", cx)),
                            )
                            .accessibility_label("Run events / Trace")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.events_open = !this.events_open;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("settings")
                            .ghost()
                            .w_full()
                            .h_10()
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child("Settings")
                                    .child(pill("Ctrl+,", cx)),
                            )
                            .accessibility_label("Settings")
                            .on_click({
                                let chat = cx.entity();
                                move |_, _, cx| Self::open_preferences(chat.clone(), cx)
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_header(&self, cx: &App) -> Div {
        let model = self
            .config
            .providers
            .get(self.settings.provider.key())
            .map_or("Unavailable", |provider| provider.model.as_str());
        let status_color = match self.session.state {
            crate::session::RunState::Failed | crate::session::RunState::LimitReached => {
                cx.theme().danger
            }
            state if state.active() => cx.theme().primary,
            _ => cx.theme().success,
        };
        div()
            .h(px(60.))
            .flex_shrink_0()
            .px_6()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .child(div().size_2().rounded_full().bg(status_color))
                            .child(format!(
                                "{:?} · Run {}",
                                self.session.state,
                                self.session.run_id()
                            )),
                    )
                    .child(div().min_w_0().truncate().child(pill(
                        format!("🌐 {:?} · {model}", self.settings.provider),
                        cx,
                    ))),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .gap_2()
                    .child(pill(
                        if self.settings.autonomous {
                            "🔒 Autonomous"
                        } else {
                            "Coding proposals"
                        },
                        cx,
                    ))
                    .when(self.settings.autonomous, |row| {
                        row.child(pill("🛠 Read-only tools", cx))
                    }),
            )
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> Div {
        let disabled = self.session.state.active() || self.saving;
        div()
            .w_full()
            .flex_1()
            .flex()
            .flex_col()
            .justify_center()
            .items_center()
            .gap_4()
            .py_8()
            .child(
                div()
                    .text_3xl()
                    .font_bold()
                    .text_center()
                    .child("What would you like to build?"),
            )
            .child(
                div()
                    .text_sm()
                    .text_center()
                    .text_color(cx.theme().muted_foreground)
                    .child("Explore an idea, solve a problem, or plan your next change."),
            )
            .child(
                div().w_full().mt_6().flex().flex_wrap().gap_4().children(
                    [
                        (
                            "idea",
                            "✦",
                            "Plan a feature",
                            "Outline a new module and its steps",
                            "Help me plan a new feature.",
                        ),
                        (
                            "explain",
                            "⌘",
                            "Explain code",
                            "Understand a complex piece of code",
                            "Help me understand a piece of code.",
                        ),
                        (
                            "review",
                            "◇",
                            "Review an approach",
                            "Explore tradeoffs before you build",
                            "Help me review my implementation approach.",
                        ),
                    ]
                    .into_iter()
                    .map(|(id, icon, title, description, prompt)| {
                        div()
                            .id(SharedString::from(format!("suggestion-{id}")))
                            .flex_1()
                            .min_w(px(150.))
                            .rounded_lg()
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().muted)
                            .relative()
                            // Relative positioning lifts the card without moving its siblings.
                            .when(!disabled, |card| {
                                card.hover(|style| {
                                    style.top(px(-4.)).border_color(cx.theme().primary)
                                })
                            })
                            .child(
                                Button::new(id)
                                    .ghost()
                                    .w_full()
                                    .h_auto()
                                    .p_4()
                                    .accessibility_label(title)
                                    .disabled(disabled)
                                    .child(
                                        div()
                                            .w_full()
                                            .flex()
                                            .flex_col()
                                            .items_start()
                                            .gap_3()
                                            .whitespace_normal()
                                            .child(
                                                div()
                                                    .text_lg()
                                                    .text_color(cx.theme().primary)
                                                    .child(icon),
                                            )
                                            .child(div().text_sm().font_semibold().child(title))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(description),
                                            ),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.input.update(cx, |input, cx| {
                                            input.set_value(prompt, window, cx)
                                        });
                                        this.input.read(cx).focus_handle(cx).focus(window, cx);
                                        cx.notify();
                                    })),
                            )
                    }),
                ),
            )
            .child(
                div()
                    .mt_4()
                    .w_full()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .text_color(cx.theme().muted_foreground)
                    .child("Example · ⚡ tool::text_stats analyzed 1 file (240 tokens)")
                    .child(div().text_color(cx.theme().success).child("✓ Validated")),
            )
    }

    fn render_composer(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let active = self.session.state.active();
        // Use the editor's actual focus handle; never create focus state in render.
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let context_bytes = self
            .session
            .retained_bytes()
            .saturating_add(self.input.read(cx).value().len());
        div().px_6().pb_5().flex_shrink_0()
            .child(div().w_full().max_w(px(800.)).mx_auto().flex().flex_col().gap_3()
                .when(!self.notice.is_empty(), |view| view.child(div().text_sm().child(self.notice.clone())))
                .child(div().id("composer").p_4().rounded_xl().border_1()
                    .border_color(cx.theme().border).bg(cx.theme().muted)
                    .when(focused, |view| view.border_color(cx.theme().primary).shadow(vec![BoxShadow {
                        color: cx.theme().primary.opacity(0.15),
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(16.),
                        spread_radius: px(3.),
                        inset: false,
                    }]))
                    .child(Textarea::new(&self.input).w_full().appearance(false).bordered(false)
                        .disabled(active || self.saving).aria_label("Message OSCAR"))
                    .child(div().mt_3().flex().flex_wrap().justify_between().items_center().gap_3()
                        .child(div().flex().flex_wrap().gap_3().text_xs().text_color(cx.theme().muted_foreground)
                            .child(if self.settings.autonomous { "🛠 Read-only tools enabled" } else { "Proposal mode" })
                            .child(div().font_family(cx.theme().mono_font_family.clone())
                                .child(format!("Context: {context_bytes} / {} bytes", self.config.limits.max_input_bytes))))
                        .child(if active {
                            Button::new("stop").label("Stop Generation").on_click(cx.listener(|this, _, _, cx| {
                                this.session.stop(); cx.notify();
                            }))
                        } else {
                            Button::new("submit").primary().label("Send ↑").px_4().rounded_md()
                                .disabled(self.saving || self.input.read(cx).value().trim().is_empty())
                                .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))
                        })))
                .child(div().text_xs().text_center().text_color(cx.theme().muted_foreground)
                    .child("Enter inserts a new line · New conversation preserves execution budgets")))
    }

    pub(super) fn render_conversation(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .child(self.render_header(cx))
            .when(self.events_open, |view| {
                view.child(
                    div()
                        .id("runtime-events")
                        .max_h(px(160.))
                        .overflow_y_scroll()
                        .px_6()
                        .py_3()
                        .text_xs()
                        .font_family(cx.theme().mono_font_family.clone())
                        .bg(cx.theme().muted)
                        .when(self.session.runtime_events().is_empty(), |view| {
                            view.child(
                                "No runtime events yet. Autonomous runs report their phases here.",
                            )
                        })
                        .children(self.session.runtime_events().iter().map(|event| {
                            div().child(format!(
                                "#{} {}{}",
                                event.sequence,
                                event.phase,
                                event
                                    .failure
                                    .map_or(String::new(), |failure| format!(" · {failure:?}"))
                            ))
                        })),
                )
            })
            .child(
                div()
                    .id("history")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p_6()
                    .child(
                        div()
                            .w_full()
                            .min_h_full()
                            .max_w(px(800.))
                            .mx_auto()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .when(self.session.messages().is_empty(), |view| {
                                view.child(self.render_empty_state(cx))
                            })
                            .children(
                                self.session
                                    .messages()
                                    .iter()
                                    .enumerate()
                                    .map(|(i, message)| Self::message(i, message, cx)),
                            )
                            .when(!self.session.preview.is_empty(), |view| {
                                view.child(Self::message(
                                    self.session.messages().len(),
                                    &Message::Model(self.session.preview.clone()),
                                    cx,
                                ))
                            }),
                    ),
            )
            .child(self.render_composer(window, cx))
            .into_any_element()
    }
}

impl Render for Chat {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .key_context("OscarChat")
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .on_action(cx.listener(|this, _: &ToggleEvents, _, cx| {
                this.events_open = !this.events_open;
                cx.notify();
            }))
            .on_action({
                let chat = cx.entity();
                move |_: &OpenSettings, _, cx| Self::open_preferences(chat.clone(), cx)
            })
            .child(self.dock_area.clone())
    }
}
