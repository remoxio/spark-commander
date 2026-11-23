//! A saved server's details, for the sidebar's Connections section.
//! Its saved password: the eye shows it once the master password is typed
//! again, the trash can deletes it.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme, Icon, IndexPath, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::remote::connections::Connection;
use sc_core::remote::{self, Scheme};
use sc_core::remote::secrets;
use sc_core::t;

type Protocols = SelectState<Vec<&'static str>>;

type Saved = Rc<dyn Fn(Connection, &mut Window, &mut App)>;

/// A saved server's details; the password goes to the keyring.
pub(crate) fn open_edit(
    connection: Connection,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(Connection, &mut Window, &mut App) + 'static,
) {
    let build = move |main, window: &mut Window, cx: &mut App| {
    let mut input = |value: Option<String>, placeholder: &str, cx: &mut App| {
        let placeholder = placeholder.to_owned();
        cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(value.unwrap_or_default())
        })
    };
    let name = input(Some(connection.name.clone()), &t!("connections.placeholder.name"), cx);
    let host = input(
        Some(connection.host.clone()),
        &t!("connections.placeholder.server"),
        cx,
    );
    let port = input(connection.port.map(|port| port.to_string()), &t!("connections.placeholder.port"), cx);
    let user = input(
        connection.user.clone(),
        &t!("connections.placeholder.user"),
        cx,
    );
    let path = input(connection.path.clone(), &t!("connections.placeholder.path"), cx);
    let region = input(connection.region.clone(), &t!("connections.placeholder.region"), cx);
    let kept = remote::keyring_key(&connection.address());
    let saved = secrets::is_saved(&kept).then_some(kept);
    let password = cx.new(|cx| {
        InputState::new(window, cx).masked(true).placeholder(match saved {
            Some(_) => t!("connections.placeholder.password_saved"),
            None => t!("connections.placeholder.password"),
        })
    });
    let at = Scheme::ALL.iter().position(|scheme| *scheme == connection.scheme);
    let labels = Scheme::ALL.map(Scheme::label).to_vec();
    let protocol = cx.new(|cx| SelectState::new(labels, at.map(IndexPath::new), window, cx));
    let view = cx.new(|cx| EditConnection {
        scheme: connection.scheme,
        _picked: cx.subscribe_in(&protocol, window, |this: &mut EditConnection, _, event: &SelectEvent<Vec<&'static str>>, window, cx| {
            let SelectEvent::Confirm(Some(label)) = event else { return };
            if let Some(scheme) = Scheme::ALL.into_iter().find(|scheme| scheme.label() == *label) {
                this.scheme = scheme;
                // The service's own sign-in server, unless one is typed.
                let server = match scheme {
                    Scheme::B2 => "api.backblazeb2.com",
                    Scheme::Swift => "auth.api.rackspacecloud.com",
                    _ => "",
                };
                let host = &this.fields[1];
                if !server.is_empty() && host.read(cx).value().trim().is_empty() {
                    host.update(cx, |input, cx| input.set_value(server, window, cx));
                }
                cx.notify();
            }
        }),
        protocol,
        fields: [name.clone(), host, port, user, path],
        region,
        password,
        saved,
        shown: None,
        folder: connection.folder.clone(),
        done: Rc::new(done),
        main,
    });
    crate::dialogs::pack_dialog::focus_all(&name, window, cx);
    view
    };
    // It closes itself once the server is given.
    let ok = |view: &Entity<EditConnection>, window: &mut Window, cx: &mut App| {
        view.update(cx, |this, cx| this.finish(window, cx));
        false
    };
    crate::dialogs::dialog_window::show(t!("connections.dialog.title"), size(px(560.), px(400.)), window, cx, build, ok);
}

struct EditConnection {
    scheme: Scheme,
    protocol: Entity<Protocols>,
    _picked: Subscription,
    /// Name, host, port, user, path.
    fields: [Entity<InputState>; 5],
    /// S3's alone.
    region: Entity<InputState>,
    password: Entity<InputState>,
    /// The saved password's key, while there is one.
    saved: Option<String>,
    /// The saved password, once the eye has shown it.
    shown: Option<String>,
    /// The list's folder it is in, kept.
    folder: Option<String>,
    done: Saved,
    main: Option<crate::dialogs::dialog_window::Main>,
}

impl EditConnection {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let [name, host, port, user, path] = self
            .fields
            .each_ref()
            .map(|input| input.read(cx).value().trim().to_string());
        if host.is_empty() {
            return window.push_notification(Notification::error(t!("connections.error.no_server")), cx);
        }
        let port = match port.as_str() {
            "" => None,
            text => match text.parse() {
                Ok(port) => Some(port),
                Err(_) => {
                    return window
                        .push_notification(Notification::error(t!("connections.error.port", text = text)), cx);
                }
            },
        };
        let some = |text: String| (!text.is_empty()).then_some(text);
        let connection = Connection {
            name,
            scheme: self.scheme,
            host,
            port,
            user: some(user),
            path: some(path),
            folder: self.folder.clone(),
            region: (self.scheme == Scheme::S3).then(|| self.region.read(cx).value().trim().to_string()).and_then(some),
        };
        let password = self.password.read(cx).value().to_string();
        let key = remote::keyring_key(&connection.address());
        // The one the eye showed is saved already.
        let unchanged = self.shown.as_ref() == Some(&password) && self.saved.as_ref() == Some(&key);
        let keep = self.scheme.logs_in() && !password.is_empty() && !unchanged;
        let done = self.done.clone();
        crate::dialogs::dialog_window::close(window, cx);
        crate::dialogs::dialog_window::answer(self.main, window, cx, move |window, cx| {
            done(connection, window, cx);
            if keep {
                super::passwords_dialog::keep(key, password, window, cx);
            }
        });
    }

    /// Shows or hides what is typed; an empty field, the saved password,
    /// after the master password.
    fn eye(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.password.read(cx);
        let masked = input.presentation().is_masked();
        let Some(key) = self.saved.clone().filter(|_| masked && input.value().is_empty()) else {
            return self.password.update(cx, |input, cx| input.set_masked(!masked, window, cx));
        };
        let view = cx.entity().downgrade();
        let show = move |window: &mut Window, cx: &mut App| {
            let key = key.clone();
            // The keyring may wait.
            let read = cx.background_spawn(async move { secrets::password(&key) });
            let view = view.clone();
            window
                .spawn(cx, async move |cx| {
                    let password = read.await;
                    let _ = view.update_in(cx, |this, window, cx| match password {
                        Some(password) => {
                            this.password.update(cx, |input, cx| {
                                input.set_value(password.clone(), window, cx);
                                input.set_masked(false, window, cx);
                            });
                            this.shown = Some(password);
                        }
                        None => window.push_notification(Notification::error(t!("connections.password.not_read")), cx),
                    });
                })
                .detach();
        };
        use super::passwords_dialog::{Ask, ask};
        // An earlier version's, saved without one: a master password is set first.
        ask(if secrets::has_master() { Ask::Reveal } else { Ask::First }, window, cx, show);
    }

    /// Deletes the saved password, once that is confirmed.
    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        let (title, text, button) = (t!("connections.password.delete_title"), t!("connections.password.delete_confirm"), t!("connections.password.delete_button"));
        crate::dialogs::dialog_window::question(title, text, button, true, window, cx, move |window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        if let Some(key) = this.saved.take() {
                            cx.background_spawn(async move { secrets::forget(&key) }).detach();
                        }
                        this.shown = None;
                        this.password.update(cx, |input, cx| {
                            input.set_value("", window, cx);
                            input.set_masked(true, window, cx);
                            input.set_placeholder(t!("connections.placeholder.password"), window, cx);
                        });
                        cx.notify();
                    });
        });
    }
}

impl Render for EditConnection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (labels, secret) = match self.scheme {
            Scheme::S3 => (
                [t!("connections.field.name"), t!("connections.field.endpoint"), t!("connections.field.port"), t!("connections.field.access_key"), t!("connections.field.bucket")],
                t!("connections.field.secret_key"),
            ),
            Scheme::B2 => (
                [t!("connections.field.name"), t!("connections.field.server"), t!("connections.field.port"), t!("connections.field.key_id"), t!("connections.field.bucket")],
                t!("connections.field.application_key"),
            ),
            Scheme::Swift => (
                [t!("connections.field.name"), t!("connections.field.server"), t!("connections.field.port"), t!("connections.field.user"), t!("connections.field.container")],
                t!("connections.field.api_key"),
            ),
            _ => (
                [t!("connections.field.name"), t!("connections.field.server"), t!("connections.field.port"), t!("connections.field.user"), t!("connections.field.start_folder")],
                t!("connections.field.password"),
            ),
        };
        let field = |label: String, input: &Entity<InputState>| {
            h_flex()
                .gap_3()
                .child(div().w(px(136.)).flex_none().child(label))
                .child(div().flex_1().child(Input::new(input)))
        };
        let [name, host, port, user, path] = &self.fields;
        let [first, server_label, port_label, user_label, path_label] = labels;
        v_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .child(field(first, name))
            .child(
                h_flex()
                    .gap_3()
                    .child(div().w(px(136.)).flex_none().child(t!("connections.field.protocol")))
                    .child(div().flex_1().child(Select::new(&self.protocol))),
            )
            .child(field(server_label, host))
            .child(field(port_label, port))
            .child(field(user_label, user))
            // The password right under its username.
            .when(self.scheme.logs_in(), |el| {
                let masked = self.password.read(cx).presentation().is_masked();
                let theme = cx.theme();
                // Joined to the field's right edge as Windows' text box buttons: its height inside the frame (32 px, macOS's 24), past its 10 px padding.
                let mac = crate::appearance::mac_tabs(cx).is_some();
                let (side, corner, line) = (px(if mac { 22. } else { 30. }), if mac { px(5.) } else { theme.radius - px(1.) }, theme.input);
                let last = self.saved.is_none();
                let button = |id: &'static str, icon: Lucide, last: bool| {
                    Button::new(id)
                        .ghost()
                        .icon(Icon::new(icon).size_4())
                        .tab_stop(false)
                        .w(side)
                        .h(side)
                        .rounded_none()
                        .when(last, |el| el.rounded_r(corner))
                        .border_0()
                        .border_l_1()
                        .border_color(line)
                };
                let eye = button("password-eye", if masked { Lucide::Eye } else { Lucide::EyeOff }, last)
                    .on_click(cx.listener(|this, _, window, cx| this.eye(window, cx)));
                let trash = self.saved.is_some().then(|| {
                    button("password-delete", Lucide::Trash, true)
                        .tooltip(t!("connections.password.delete"))
                        .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx)))
                });
                let buttons = h_flex().mr(px(-10.)).child(eye).children(trash);
                el.child(
                    h_flex()
                        .gap_3()
                        .child(div().w(px(136.)).flex_none().child(secret))
                        .child(div().flex_1().child(Input::new(&self.password).suffix(buttons))),
                )
            })
            .child(field(path_label, path))
            .when(self.scheme == Scheme::S3, |el| el.child(field(t!("connections.field.region"), &self.region)))
            .child(
                crate::dialogs::footer(cx)
                    .children(crate::dialogs::ok_cancel(
                        Button::new("connection-ok")
                            .primary()
                            .label(t!("button.save"))
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| this.finish(window, cx))),
                        Button::new("connection-cancel")
                            .outline()
                            .label(t!("remote.dialog.cancel"))
                            .min_w(px(80.))
                            .on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx)),
                    )),
            )
    }
}
