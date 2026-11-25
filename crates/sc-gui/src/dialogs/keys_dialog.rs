//! Configuration > Shortcuts: each command's keys, a preset's, changed one by one.
use gpui_kit::component::scroll::{ScrollableElement as _, Scrollbar};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, IndexPath, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::keymap::{self, Choice, KeySettings, Preset};
use sc_core::menu::{self, Command};

use crate::{Commander, dialogs::{Role, name_dialog}};

type Choices = SelectState<Vec<SharedString>>;

/// The presets offered; Midnight Commander's waits for its keys to be right.
const PRESETS: [Preset; 4] = [Preset::SparkCommander, Preset::TotalCommander, Preset::Krusader, Preset::ForkLift];

/// The dropdown's item for keys of no preset or set.
fn custom() -> SharedString {
    sc_core::t!("keys.custom").into()
}

/// Settings' size, its window 720 by 577 px on Xfce.
const WIDTH: f32 = 720.;

/// GNOME's: each change applied at once, as its preferences; Reset All and a × beside the title.
fn instant() -> bool {
    crate::dialogs::dialog_window::separate() && sc_core::system::theme::control_style() == Some(sc_core::system::theme::ControlStyle::Adwaita)
}
const BODY_HEIGHT: f32 = 500.;
const KEY_WIDTH: f32 = 150.;
/// A row's reset button's column.
const RESET_WIDTH: f32 = 22.;
/// Keys that only change others; a shortcut needs one more.
const MODIFIERS: [&str; 9] = [
    "", "shift", "control", "ctrl", "alt", "platform", "cmd", "super", "function",
];

/// A key as keyboards label it, Ctrl+Shift+F4; a chord's keys one after another.
pub(crate) fn shown(key: &str) -> String {
    let strokes = key.split_whitespace().map(|stroke| {
        Keystroke::parse(stroke).map_or_else(|_| stroke.to_owned(), |key| Kbd::format(&key))
    });
    strokes.collect::<Vec<_>>().join(" ")
}

pub fn open(
    commander: WeakEntity<Commander>,
    keys: KeySettings,
    window: &mut Window,
    cx: &mut App,
) {
    let build = move |main, window: &mut Window, cx: &mut App| {
    let search = cx.new(|cx| InputState::new(window, cx).placeholder(sc_core::t!("keys.search.placeholder")));
    let (items, label) = choices(&keys);
    let at = items.iter().position(|item| *item == label);
    let choices = cx.new(|cx| SelectState::new(items.clone(), at.map(IndexPath::new), window, cx));
    let view = cx.new(|cx| {
        let this = cx.entity().downgrade();
        // Before the key bindings, so a captured key runs nothing.
        let capture = cx.intercept_keystrokes(move |event, _, cx| {
            let key = &event.keystroke;
            let waiting = this.read_with(cx, |this: &KeysDialog, _| this.capturing.is_some()).unwrap_or(false);
            if waiting && !MODIFIERS.contains(&key.key.as_str()) {
                cx.stop_propagation();
                let _ = this.update(cx, |this, cx| this.captured(key, cx));
            }
        });
        let chosen = cx.subscribe(&choices, |this: &mut KeysDialog, _, event: &SelectEvent<Vec<SharedString>>, cx| {
            let SelectEvent::Confirm(Some(label)) = event else { return };
            this.choose(label, cx);
        });
        KeysDialog {
            main: None,
            commander,
            saved: keys.clone(),
            draft: keys,
            choices,
            _chosen: chosen,
            synced: (items, label),
            _search: cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    cx.notify()
                }
            }),
            search,
            capturing: None,
            _capture: capture,
            note: None,
            groups: menu::groups(),
        }
    });
    view.update(cx, |this, _| this.main = main);
    view
    };
    // GNOME: its title beside Reset All and its × (`render`).
    if instant() {
        return crate::dialogs::dialog_window::open_headed(sc_core::t!("keys.title"), size(px(WIDTH), px(BODY_HEIGHT)), window, cx, move |main, window, cx| (build(Some(main), window, cx), None));
    }
    crate::dialogs::dialog_window::show(sc_core::t!("keys.title"), size(px(WIDTH), px(BODY_HEIGHT)), window, cx, build, |_, _, _| false);
}

/// The dropdown's items (the presets, the user's sets, Custom while the
/// keys are neither) and the one the keys are.
fn choices(keys: &KeySettings) -> (Vec<SharedString>, SharedString) {
    let choice = keys.choice();
    let mut items: Vec<SharedString> = PRESETS.iter().map(|preset| preset.name().into()).collect();
    if let Choice::Preset(preset) = &choice
        && !PRESETS.contains(preset)
    {
        items.push(preset.name().into());
    }
    items.extend(keys.saved.keys().map(|name| SharedString::from(name.clone())));
    if choice == Choice::Custom {
        items.push(custom());
    }
    let label = match choice {
        Choice::Preset(preset) => preset.name().into(),
        Choice::Saved(name) => name.into(),
        Choice::Custom => custom(),
    };
    (items, label)
}

struct KeysDialog {
    /// In a window of its own: the main window, whose keys they are.
    main: Option<crate::dialogs::dialog_window::Main>,
    commander: WeakEntity<Commander>,
    /// As last applied.
    saved: KeySettings,
    draft: KeySettings,
    choices: Entity<Choices>,
    _chosen: Subscription,
    /// What the dropdown shows, put right as the keys change.
    synced: (Vec<SharedString>, SharedString),
    search: Entity<InputState>,
    _search: Subscription,
    /// The slot waiting for a key: a command's shortcut (0) or alternate (1).
    capturing: Option<(Command, usize)>,
    _capture: Subscription,
    /// What the last change did.
    note: Option<(SharedString, bool)>,
    groups: Vec<(String, Vec<(String, Command)>)>,
}

impl KeysDialog {
    fn label(&self, command: Command) -> &str {
        let mut all = self.groups.iter().flat_map(|(_, commands)| commands);
        all.find(|(_, c)| *c == command).map_or("", |(label, _)| label.as_str())
    }

    fn captured(&mut self, key: &Keystroke, cx: &mut Context<Self>) {
        let Some((command, slot)) = self.capturing.take() else {
            return;
        };
        cx.notify();
        if key.key == "escape" && !key.modifiers.modified() {
            self.note = None;
            return;
        }
        let key = key.unparse();
        let taken = self.draft.set(command, slot, key.clone());
        let name = shown(&key);
        self.note = Some(match (keymap::taken_by(&key, command), taken) {
            (Some(by), _) => (sc_core::t!("keys.note.taken_by_system", key = name, by = by).into(), true),
            (None, Some(other)) => {
                let text = sc_core::t!("keys.note.moved", key = name, command = self.label(command), other = self.label(other));
                (text.into(), false)
            }
            (None, None) => (sc_core::t!("keys.note.set", key = name, command = self.label(command)).into(), false),
        });
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft == self.saved {
            return;
        }
        let (keys, commander) = (self.draft.clone(), self.commander.clone());
        crate::dialogs::dialog_window::answer(self.main, window, cx, move |window, cx| {
            let _ = commander.update(cx, |commander, cx| {
                let mut settings = commander.settings.clone();
                settings.keys = keys;
                commander.apply_settings(settings, window, cx)
            });
        });
        self.saved = self.draft.clone();
        cx.notify();
    }

    /// A preset or a saved set from the dropdown; Custom is the keys as they are.
    fn choose(&mut self, label: &str, cx: &mut Context<Self>) {
        let dropped = self.draft.choice() == Choice::Custom;
        let preset = Preset::ALL.into_iter().find(|preset| preset.name() == label);
        match preset {
            Some(preset) => self.draft.use_preset(preset),
            None if self.draft.saved.contains_key(label) => self.draft.use_saved(label),
            None => return,
        }
        self.note = dropped.then(|| (sc_core::t!("keys.note.custom_replaced", name = label).into(), false));
        cx.notify();
    }

    /// The keys as they are, under a name of the user's.
    fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = match self.draft.choice() {
            Choice::Saved(name) => name,
            _ => sc_core::t!("keys.save_as.default_name"),
        };
        let this = cx.entity().downgrade();
        let (title, label) = (sc_core::t!("keys.save_as.title"), sc_core::t!("keys.save_as.label"));
        name_dialog::open(title, label, sc_core::t!("button.save"), name, window, cx, move |name, _, cx| {
            let name = name.trim().to_owned();
            let _ = this.update(cx, |this, cx| {
                let reserved = name.is_empty() || name == custom() || Preset::ALL.iter().any(|preset| preset.name() == name);
                this.note = Some(match reserved {
                    true => (sc_core::t!("keys.note.name_reserved", name = name).into(), true),
                    false => {
                        this.draft.save_as(&name);
                        (sc_core::t!("keys.note.saved", name = name).into(), false)
                    }
                });
                cx.notify();
            });
        });
    }

    fn render_presets(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let saved = match self.draft.choice() {
            Choice::Saved(name) => Some(name),
            _ => None,
        };
        h_flex()
            .gap_2()
            .child(sc_core::t!("keys.set_label"))
            .child(div().w(px(240.)).child(Select::new(&self.choices)))
            .child(
                Button::new("keys-save-as")
                    .outline()
                    .label(sc_core::t!("keys.save_as.button"))
                    .on_click(cx.listener(|this, _, window, cx| this.save_as(window, cx))),
            )
            .child(
                Button::new("keys-delete-set")
                    .outline()
                    .label(sc_core::t!("keys.delete_set"))
                    .disabled(saved.is_none())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(name) = &saved {
                            this.draft.delete_saved(name);
                            this.note = Some((sc_core::t!("keys.note.deleted", name = name).into(), false));
                            cx.notify();
                        }
                    })),
            )
    }

    fn render_key(&self, command: Command, slot: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let key = self.draft.keys_of(command)[slot].clone();
        let waiting = self.capturing == Some((command, slot));
        let taken = keymap::taken_by(&key, command).filter(|_| !key.is_empty());
        let theme = cx.theme();
        let name = ["key-0", "key-1"][slot];
        let text = match waiting {
            true => sc_core::t!("keys.press_key"),
            false => shown(&key),
        };
        let cell = h_flex()
            .id((name, command as usize))
            .flex_1()
            .min_w_0()
            .h(px(24.))
            .px_2()
            .gap_1()
            .border_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .border_color(if waiting { theme.primary } else { theme.border })
            .when(waiting, |el| el.bg(theme.primary.opacity(0.15)))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .when(taken.is_some(), |el| el.text_color(theme.danger))
                    .child(text),
            )
            .when_some(taken, |el, by| {
                el.child(Icon::new(IconName::TriangleAlert).small().text_color(theme.warning))
                    .tooltip(move |window, cx| Tooltip::new(sc_core::t!("keys.taken_by.tooltip", by = by)).build(window, cx))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.capturing = Some((command, slot));
                this.note = Some((sc_core::t!("keys.note.press_new_key").into(), false));
                cx.notify();
            }));
        let clear = Button::new((["clear-0", "clear-1"][slot], command as usize))
            .ghost()
            .xsmall()
            .icon(IconName::Close)
            .tooltip(sc_core::t!("keys.clear.tooltip"))
            .disabled(key.is_empty())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.draft.set(command, slot, String::new());
                cx.notify();
            }));
        h_flex().w(px(KEY_WIDTH)).gap_1().child(cell).child(clear)
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_lowercase();
        let matches = |label: &str, command: Command| {
            let keys = self.draft.keys_of(command).map(|key| shown(&key).to_lowercase());
            query.is_empty()
                || label.to_lowercase().contains(&query)
                || keys.iter().any(|key| key.contains(&query))
        };
        let mut list = v_flex();
        for (group, commands) in &self.groups {
            let rows: Vec<_> = commands.iter().filter(|(label, c)| matches(label, *c)).collect();
            if rows.is_empty() {
                continue;
            }
            let theme = cx.theme();
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .bg(theme.muted)
                    .text_color(theme.muted_foreground)
                    .child(group.clone()),
            );
            for (label, command) in rows.iter().map(|row| (row.0.clone(), row.1)) {
                let changed = self.draft.is_changed(command);
                let reset = Button::new(("reset", command as usize))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Undo2)
                    .tooltip(sc_core::t!("keys.reset.tooltip", preset = self.draft.preset.name()))
                    .disabled(!changed)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.draft.reset(command);
                        cx.notify();
                    }));
                list = list.child(
                    h_flex()
                        .h(px(30.))
                        .px_2()
                        .gap_2()
                        .child(div().flex_1().min_w_0().truncate().child(label))
                        .child(self.render_key(command, 0, cx))
                        .child(self.render_key(command, 1, cx))
                        .child(div().w(px(RESET_WIDTH)).flex_none().child(reset)),
                );
            }
        }
        let theme = cx.theme();
        v_flex()
            .flex_1()
            .min_h_0()
            .border_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1().child(sc_core::t!("keys.column.command")))
                    // Over the keys' text, inside their boxes' 1 px border and 8 px padding.
                    .child(div().w(px(KEY_WIDTH)).pl(px(9.)).child(sc_core::t!("keys.column.shortcut")))
                    .child(div().w(px(KEY_WIDTH)).pl(px(9.)).child(sc_core::t!("keys.column.alternate")))
                    // Over the reset buttons and the list's room for its scrollbar, without a gap between.
                    .child(div().w(px(RESET_WIDTH) + Scrollbar::width())),
            )
            .child(
                div()
                    .id("keys-list")
                    .flex_1()
                    .min_h_0()
                    // The scrollbar beside the rows' buttons, not over them.
                    .pr(Scrollbar::width())
                    .overflow_y_scrollbar()
                    .child(list),
            )
    }
}

impl Render for KeysDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The dropdown follows the keys: Custom once one changes.
        let wanted = choices(&self.draft);
        if wanted != self.synced {
            let (items, label) = wanted.clone();
            self.choices.update(cx, |select, cx| {
                select.set_items(items, window, cx);
                select.set_selected_value(&label, window, cx);
            });
            self.synced = wanted;
        }
        let changed = self.draft != self.saved;
        // GNOME: a change applied as made.
        let instant = instant();
        if instant && changed {
            cx.defer_in(window, |this, window, cx| this.apply(window, cx));
        }
        let note = self.note.clone().map(|(text, warn)| {
            let theme = cx.theme();
            div()
                .flex_1()
                .min_w_0()
                .text_color(if warn { theme.danger } else { theme.muted_foreground })
                .child(text)
        });
        let reset_all = Button::new("keys-reset-all")
            .outline()
            .label(sc_core::t!("keys.reset_all"))
            .disabled(self.draft.choice() != Choice::Custom)
            .on_click(cx.listener(|this, _, _, cx| {
                // Back to the set chosen, else the preset.
                match this.draft.set.clone().filter(|name| this.draft.saved.contains_key(name)) {
                    Some(name) => this.draft.use_saved(&name),
                    None => this.draft.changed.clear(),
                }
                this.note = None;
                cx.notify();
            }));
        if instant {
            let close = Button::new("keys-close")
                .ghost()
                .small()
                .icon(IconName::Close)
                .rounded_full()
                .on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx));
            let title = div().flex_1().text_xl().font_weight(FontWeight::SEMIBOLD).child(sc_core::t!("keys.title"));
            return v_flex()
                .w_full()
                .h(px(BODY_HEIGHT))
                .gap_3()
                .text_sm()
                .child(h_flex().gap_2().items_center().child(title).child(reset_all).child(close))
                .child(self.render_presets(cx))
                .child(Input::new(&self.search).small())
                .child(self.render_list(cx))
                .children(note)
                .into_any_element();
        }
        v_flex()
            .w_full()
            .h(px(BODY_HEIGHT))
            .gap_3()
            .text_sm()
            .child(self.render_presets(cx))
            .child(Input::new(&self.search).small())
            .child(self.render_list(cx))
            .child(
                crate::dialogs::footer(cx)
                    .child(div().flex_1().min_w_0().children(note))
                    .child(reset_all)
                    .children(crate::dialogs::ordered([
                        (Role::Main, Button::new("keys-ok")
                            .primary()
                            .label(sc_core::t!("keys.ok"))
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply(window, cx);
                                crate::dialogs::dialog_window::close(window, cx);
                            }))),
                        (Role::Dismiss, Button::new("keys-cancel")
                            .outline()
                            .label(sc_core::t!("keys.cancel"))
                            .min_w(px(80.))
                            .on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx))),
                        (Role::Apply, Button::new("keys-apply")
                            .outline()
                            .label(sc_core::t!("keys.apply"))
                            .min_w(px(80.))
                            .disabled(!changed)
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx)))),
                    ])),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{KeySettings, Keystroke, Preset};

    #[test]
    fn every_preset_key_parses() {
        for preset in Preset::ALL {
            let keys = KeySettings { preset, ..Default::default() };
            for (key, _) in keys.bindings() {
                for stroke in key.split_whitespace() {
                    assert!(Keystroke::parse(stroke).is_ok(), "{}: {key}", preset.name());
                }
            }
        }
    }
}
