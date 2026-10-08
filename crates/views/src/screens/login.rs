use gpui::prelude::*;
use gpui::{
    Context, Entity, FontWeight, IntoElement, Pixels, Render, SharedString, Window, div, px, svg,
};
use i18n::t;
use music::SignIn;
use state::{Session, SessionState, Sonora, Usage};
use ui::ActiveTheme as _;
use ui::{Button, Checkbox, Input, Modal, TabBar, Text};

const COLUMN: Pixels = px(280.);
const LOGO: Pixels = px(48.);

struct Column {
    slug: &'static str,
    name: &'static str,
    options: Vec<SignIn>,
    disabled: bool,
    cancel: bool,
}

enum LoginAction {
    SignIn(SignIn),
    Credentials,
}

struct LoginOption {
    id: SharedString,
    label: SharedString,
    action: LoginAction,
    primary: bool,
}

pub struct LoginView {
    session: Entity<Session>,
    usage: Entity<Usage>,
    server: Entity<Input>,
    username: Entity<Input>,
    password: Entity<Input>,
    credentials_for: Option<&'static str>,
    tab: usize,
}

impl LoginView {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let usage = Sonora::global(cx).usage.clone();
        cx.observe(&usage, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            usage,
            server: cx.new(|cx| Input::new("login-server-hint", cx)),
            username: cx.new(|cx| Input::new("login-username-hint", cx)),
            password: cx.new(|cx| Input::new("login-password-hint", cx).masked()),
            credentials_for: None,
            tab: 0,
        }
    }

    fn acted(&self, cx: &mut Context<Self>) {
        self.usage.update(cx, |usage, cx| usage.report(cx));
    }

    fn consent(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let checked = self.usage.read(cx).consented();
        Checkbox::new("usage-consent", checked)
            .label(t!("login-usage-consent"))
            .max_w(COLUMN)
            .text_color(cx.theme().muted_foreground)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.usage
                    .update(cx, |usage, cx| usage.consent(!checked, cx));
            }))
    }

    fn abandon(&mut self, cx: &mut Context<Self>) {
        self.acted(cx);
        self.clear_credentials(cx);
        self.session
            .update(cx, |session, cx| session.cancel_sign_in(cx));
    }

    fn open_credentials(&mut self, slug: &'static str, cx: &mut Context<Self>) {
        self.acted(cx);
        self.credentials_for = Some(slug);
        cx.notify();
    }

    fn clear_credentials(&mut self, cx: &mut Context<Self>) {
        self.credentials_for = None;
        self.server.update(cx, |input, cx| input.set_text("", cx));
        self.username.update(cx, |input, cx| input.set_text("", cx));
        self.password.update(cx, |input, cx| input.set_text("", cx));
    }

    fn abandon_credentials(&mut self, cx: &mut Context<Self>) {
        self.acted(cx);
        self.clear_credentials(cx);
        cx.notify();
    }

    fn submit_credentials(&mut self, cx: &mut Context<Self>) {
        let Some(slug) = self.credentials_for else {
            return;
        };
        let server = self.server.read(cx).text().to_string();
        let username = self.username.read(cx).text().to_string();
        let password = self.password.read(cx).text().to_string();
        if server.trim().is_empty() || username.trim().is_empty() || password.is_empty() {
            return;
        }
        self.clear_credentials(cx);
        self.acted(cx);
        self.session.update(cx, |session, cx| {
            session.sign_in(
                slug,
                SignIn::Credentials {
                    server,
                    username,
                    password,
                },
                cx,
            )
        });
    }

    fn option_buttons(
        &self,
        slug: &'static str,
        provider: &'static str,
        method: &SignIn,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let (suffix, action, primary) = match method {
            SignIn::Path(_) => ("-path", LoginAction::SignIn(method.clone()), false),
            SignIn::Credentials { .. } => ("-server", LoginAction::Credentials, true),
        };
        let option = LoginOption {
            id: format!("sign-in-{slug}{suffix}").into(),
            label: t!("login-sign-in", provider = provider),
            action,
            primary,
        };
        let button = Button::new(option.id)
            .label(option.label)
            .w_full()
            .disabled(disabled)
            .on_click(cx.listener(move |this, _, _, cx| match &option.action {
                LoginAction::SignIn(method) => {
                    this.acted(cx);
                    this.session
                        .update(cx, |session, cx| session.sign_in(slug, method.clone(), cx));
                }
                LoginAction::Credentials => this.open_credentials(slug, cx),
            }));
        match option.primary {
            true => button.primary(),
            false => button.outline(),
        }
    }

    fn column(&self, column: Column, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let Column {
            slug,
            name,
            options,
            disabled,
            cancel,
        } = column;
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .w(COLUMN)
            .child(
                svg()
                    .path(icons::path(crate::shared::provider_logo(slug)))
                    .size(LOGO)
                    .flex_none()
                    .text_color(theme.foreground),
            )
            .child(
                div()
                    .text_size(theme.text(Text::Large))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(SharedString::from(name.to_string())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w_full()
                    .children(
                        options
                            .iter()
                            .map(|method| self.option_buttons(slug, name, method, disabled, cx)),
                    )
                    .when(cancel, |this| {
                        this.child(
                            Button::new("cancel-sign-in")
                                .label(t!("common-cancel"))
                                .outline()
                                .w_full()
                                .on_click(cx.listener(|this, _, _, cx| this.abandon(cx))),
                        )
                    }),
            )
    }

    fn credentials_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Modal::new("server-prompt", t!("login-server-title"))
            .w(px(560.))
            .detail(t!("login-server-detail"))
            .child(self.server.clone())
            .child(self.username.clone())
            .child(self.password.clone())
            .action(
                Button::new("cancel-server")
                    .ghost()
                    .label(t!("common-cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.abandon_credentials(cx))),
            )
            .action(
                Button::new("submit-server")
                    .label(t!("login-server-submit"))
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.submit_credentials(cx))),
            )
            .on_dismiss(cx.listener(|this, _, _, cx| this.abandon_credentials(cx)))
    }
}

impl Render for LoginView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.session.read(cx).state().clone();
        let pending = self.session.read(cx).is_pending();
        let providers: Vec<state::ProviderInfo> = self.session.read(cx).providers().collect();
        let waiting = matches!(state, SessionState::Authorizing);
        let tabs = providers
            .iter()
            .enumerate()
            .map(|(index, info)| {
                Button::new(SharedString::from(format!("login-tab-{}", info.slug)))
                    .label(SharedString::from(info.name))
                    .small()
                    .ghost()
                    .selected(index == self.tab)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.acted(cx);
                        this.tab = index;
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();
        let column = providers.into_iter().nth(self.tab).map(|info| Column {
            slug: info.slug,
            name: info.name,
            options: info.options,
            disabled: pending,
            cancel: waiting && info.pending,
        });
        let failure = match &state {
            SessionState::Failed(failure) => Some(failure.clone()),
            _ => None,
        };
        let status = match &state {
            SessionState::SignedOut => t!("login-signed-out"),
            SessionState::Restoring => t!("login-restoring"),
            SessionState::Authorizing => t!("login-authorizing"),
            SessionState::SignedIn(profile) => t!("login-signed-in", name = &profile.display_name),
            SessionState::Offline(_) | SessionState::Failed(_) => t!("login-signed-out"),
        };
        let theme = *cx.theme();
        let asking = self.usage.read(cx).asking();

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let radius = crate::chrome::window_radius(Sonora::global(cx).settings.read(cx), cx);
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let radius: Option<Pixels> = None;

        div()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_6()
            .size_full()
            .when_some(radius, |this, radius| this.rounded_b(radius))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .child("Sonora")
                            .text_size(theme.text(Text::Display))
                            .font_weight(FontWeight::BOLD),
                    )
                    .child(
                        div()
                            .max_w(px(560.))
                            .text_center()
                            .text_size(theme.text(Text::Body))
                            .text_color(theme.muted_foreground)
                            .child(status),
                    ),
            )
            .when_some(failure, |this, failure| {
                this.child(crate::shared::trouble::trouble(failure, true))
            })
            .child(TabBar::new("login-providers").flex_none().items(tabs))
            .when_some(column, |this, column| this.child(self.column(column, cx)))
            .when(asking, |this| this.child(self.consent(cx)))
            .when(self.credentials_for.is_some(), |this| {
                this.child(self.credentials_prompt(cx).into_any_element())
            })
    }
}
