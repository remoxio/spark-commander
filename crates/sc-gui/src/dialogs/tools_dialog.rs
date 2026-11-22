//! Tools: the user's own command lines run from the Tools menu or their keys;
//! Tools > Manage Tools adds, changes, deletes, orders them and gives keys.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::tools::{self, Places, Tool};

use crate::{Commander, dialogs::keys_dialog, menus};

/// Keys that only change others; a tool's key needs one more.
const MODIFIERS: [&str; 9] = ["", "shift", "control", "ctrl", "alt", "platform", "cmd", "super", "function"];

impl Commander {
    /// Tool `ix`, in the active panel's folder: in the terminal, or on its own.
    pub(crate) fn run_tool(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tool) = self.settings.tools.get(ix).cloned() else {
            return;
        };
        let warn = |text: String, window: &mut Window, cx: &mut Context<Self>| window.push_notification(Notification::warning(text), cx);
        if tool.command.trim().is_empty() {
            return warn(sc_core::t!("tools.no_command", tool = tool.name), window, cx);
        }
        let panel = self.panels[self.active].tabs.current();
        if panel.remote().is_some() || panel.in_archive() {
            return warn(sc_core::t!("tools.not_local"), window, cx);
        }
        let other = self.panels[1 - self.active].tabs.current();
        let other = (other.remote().is_none() && !other.in_archive()).then(|| other.folder().to_owned());
        let folder = panel.folder().to_owned();
        let name = panel.current_entry().filter(|entry| entry.name != sc_core::PARENT).map(|entry| entry.name.clone());
        let places = Places { folder: &folder, other: other.as_deref(), name, selected: panel.marked_names() };
        if let Some(why) = tools::missing(&tool.command, &places) {
            return warn(format!("{}: {why}", tool.name), window, cx);
        }
        let line = tools::expand(&tool.command, &places);
        match tool.terminal {
            true => self.run_in_terminal_tab(line, folder, window, cx),
            false => self.start_command(&line, &folder, window, cx),
        }
    }

    /// In the shown terminal, shown first; a new shell where there is none of
    /// this computer, which reads the line once it starts.
    fn run_in_terminal_tab(&mut self, line: String, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !self.terminals.is_empty() && !self.terminal_shown {
            self.toggle_terminal(window, cx);
        }
        if !self.terminals.is_empty() && self.run_in_terminal(&line, &folder, window, cx).is_some() {
            return;
        }
        self.new_terminal_tab(window, cx);
        let Some(view) = self.terminals.last().map(|tab| tab.view.clone()) else {
            return;
        };
        // Once the shell has shown its prompt, else the line is echoed twice; at most 2 s.
        cx.spawn_in(window, async move |_, cx| {
            for _ in 0..40 {
                if view.read_with(cx, |view, _| view.started()) {
                    break;
                }
                cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
            }
            view.update(cx, |view, _| view.send(&format!("{line}\r")));
        })
        .detach();
    }

    /// The tools as now, saved; the menu and keys follow.
    fn set_tools(&mut self, tools: Vec<Tool>, window: &mut Window, cx: &mut Context<Self>) {
        let before = std::mem::replace(&mut self.settings.tools, tools);
        let keys = |tools: &[Tool]| tools.iter().map(|tool| tool.key.clone()).collect::<Vec<_>>();
        if keys(&before) != keys(&self.settings.tools) {
            menus::bind_tools(&self.settings.tools, &before, cx);
        }
        menus::set_tools(&self.settings.tools);
        self.reload_menus(cx);
        self.save_settings(window, cx);
    }

    pub(crate) fn manage_tools(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commander = cx.entity().downgrade();
        let tools = self.settings.tools.clone();
        let build = move |_, window: &mut Window, cx: &mut App| {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(sc_core::t!("tools.name.placeholder")));
        let command = cx.new(|cx| InputState::new(window, cx).placeholder("meld \"%P%N\" \"%T%N\""));
        let view = cx.new(|cx| {
            let this = cx.entity().downgrade();
            // Before the key bindings, so the key captured runs nothing.
            let capture = cx.intercept_keystrokes(move |event, window, cx| {
                let key = &event.keystroke;
                let waiting = this.read_with(cx, |this: &ToolsDialog, _| this.capturing).unwrap_or(false);
                if waiting && !MODIFIERS.contains(&key.key.as_str()) {
                    cx.stop_propagation();
                    let _ = this.update(cx, |this, cx| this.captured(key, window, cx));
                }
            });
            let edited = |field: fn(&mut Tool, String)| {
                move |this: &mut ToolsDialog, input: &Entity<InputState>, event: &InputEvent, window: &mut Window, cx: &mut Context<ToolsDialog>| {
                    if let InputEvent::Change = event {
                        let text = input.read(cx).value().to_string();
                        this.edit(window, cx, |tool| field(tool, text));
                    }
                }
            };
            let events = [
                cx.subscribe_in(&name, window, edited(|tool, text| tool.name = text)),
                cx.subscribe_in(&command, window, edited(|tool, text| tool.command = text)),
            ];
            ToolsDialog { commander, tools, selected: None, name, command, capturing: false, note: None, _capture: capture, _events: events }
        });
        view.update(cx, |this, cx| this.select(Some(0), window, cx));
        view
        };
        crate::dialogs::dialog_window::show(sc_core::t!("tools.title"), size(px(680.), px(420.)), window, cx, build, |_, _, _| false);
    }
}

struct ToolsDialog {
    commander: WeakEntity<Commander>,
    /// What the dialog shows; each change goes to the settings at once.
    tools: Vec<Tool>,
    selected: Option<usize>,
    name: Entity<InputState>,
    command: Entity<InputState>,
    /// The next key pressed is the selected tool's.
    capturing: bool,
    note: Option<SharedString>,
    _capture: Subscription,
    _events: [Subscription; 2],
}

impl ToolsDialog {
    fn commit(&self, window: &mut Window, cx: &mut Context<Self>) {
        let tools = self.tools.clone();
        let _ = self.commander.update(cx, |this, cx| this.set_tools(tools, window, cx));
    }

    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>, change: impl FnOnce(&mut Tool)) {
        let Some(tool) = self.selected.and_then(|ix| self.tools.get_mut(ix)) else {
            return;
        };
        let before = tool.clone();
        change(tool);
        if *tool != before {
            self.commit(window, cx);
            cx.notify();
        }
    }

    /// Its fields show the tool chosen.
    fn select(&mut self, ix: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = ix.filter(|&ix| ix < self.tools.len());
        (self.capturing, self.note) = (false, None);
        let tool = self.selected.map(|ix| self.tools[ix].clone()).unwrap_or_default();
        self.name.update(cx, |input, cx| input.set_value(tool.name, window, cx));
        self.command.update(cx, |input, cx| input.set_value(tool.command, window, cx));
        cx.notify();
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names: Vec<&str> = self.tools.iter().map(|tool| tool.name.as_str()).collect();
        // As many numbers as tools and one: one is free, and a message without {number} cannot loop.
        let name = (1..=self.tools.len() + 1).map(|n| sc_core::t!("tools.new_name", number = n)).find(|name| !names.contains(&name.as_str())).unwrap_or_default();
        self.tools.push(Tool { name, ..Tool::default() });
        self.commit(window, cx);
        self.select(Some(self.tools.len() - 1), window, cx);
        self.name.update(cx, |input, cx| input.focus(window, cx));
    }

    fn remove(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.tools.remove(ix);
        self.commit(window, cx);
        let next = (!self.tools.is_empty()).then(|| ix.min(self.tools.len() - 1));
        self.select(next, window, cx);
    }

    fn move_to(&mut self, ix: usize, to: usize, window: &mut Window, cx: &mut Context<Self>) {
        if to < self.tools.len() {
            self.tools.swap(ix, to);
            self.commit(window, cx);
            self.select(Some(to), window, cx);
        }
    }

    fn captured(&mut self, key: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        self.capturing = false;
        cx.notify();
        if key.key == "escape" && !key.modifiers.modified() {
            return;
        }
        let key = key.unparse();
        let shown = keys_dialog::shown(&key);
        let command = self.commander.read_with(cx, |this, _| this.settings.keys.command_of(&key)).ok().flatten();
        let other = self.tools.iter().enumerate().find(|(ix, tool)| Some(*ix) != self.selected && tool.key == key).map(|(_, tool)| tool.name.clone());
        self.note = match (command, other) {
            (_, Some(other)) => Some(sc_core::t!("tools.note.key_of_other_tool", key = shown, tool = other).into()),
            (Some(command), None) => Some(sc_core::t!("tools.note.key_of_command", key = shown, command = sc_core::menu::name(command)).into()),
            (None, None) => None,
        };
        self.edit(window, cx, |tool| tool.key = key);
    }
}

impl Render for ToolsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let selected = self.selected;
        let mut list = v_flex().id("tool-list").h(px(180.)).overflow_y_scroll().p_1().border_1().border_color(theme.border).rounded(theme.radius);
        if self.tools.is_empty() {
            list = list.child(div().p_2().text_color(theme.muted_foreground).child(sc_core::t!("tools.empty")));
        }
        for (ix, tool) in self.tools.iter().enumerate() {
            let key = (!tool.key.is_empty()).then(|| keys_dialog::shown(&tool.key));
            list = list.child(
                h_flex()
                    .id(("tool-row", ix))
                    .flex_none()
                    .h(px(28.))
                    .px_1p5()
                    .gap_2()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .when(selected == Some(ix), |el| el.bg(theme.list_active))
                    .when(selected != Some(ix), |el| el.hover(|style| style.bg(theme.list_hover)))
                    .child(div().flex_none().max_w(relative(0.4)).truncate().child(tool.name.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_xs().text_color(theme.muted_foreground).child(tool.command.clone()))
                    .children(key.map(|key| div().flex_none().text_xs().text_color(theme.muted_foreground).child(key)))
                    .on_click(cx.listener(move |this, _, window, cx| this.select(Some(ix), window, cx))),
            );
        }
        let count = self.tools.len();
        let on_selected = |button: Button, run: fn(&mut Self, usize, &mut Window, &mut Context<Self>)| {
            button.outline().disabled(selected.is_none()).on_click(cx.listener(move |this, _, window, cx| {
                if let Some(ix) = this.selected {
                    run(this, ix, window, cx);
                }
            }))
        };
        let buttons = h_flex()
            .gap_2()
            .child(Button::new("tools-new").outline().label(sc_core::t!("tools.new")).on_click(cx.listener(|this, _, window, cx| this.add(window, cx))))
            .child(on_selected(Button::new("tools-delete").label(sc_core::t!("tools.delete")), Self::remove))
            .child(on_selected(Button::new("tools-up").icon(IconName::ArrowUp).tooltip(sc_core::t!("tools.move_up")), |this, ix, window, cx| this.move_to(ix, ix.wrapping_sub(1), window, cx)).disabled(selected.is_none_or(|ix| ix == 0)))
            .child(on_selected(Button::new("tools-down").icon(IconName::ArrowDown).tooltip(sc_core::t!("tools.move_down")), |this, ix, window, cx| this.move_to(ix, ix + 1, window, cx)).disabled(selected.is_none_or(|ix| ix + 1 >= count)));
        let tool = selected.and_then(|ix| self.tools.get(ix)).cloned().unwrap_or_default();
        let none = selected.is_none();
        let label = |text: String| div().w(px(80.)).flex_none().text_color(theme.muted_foreground).child(text);
        let key = match (self.capturing, tool.key.is_empty()) {
            (true, _) => sc_core::t!("tools.key.waiting"),
            (false, true) => sc_core::t!("tools.key.none"),
            (false, false) => keys_dialog::shown(&tool.key),
        };
        let editor = v_flex()
            .gap_2()
            .child(h_flex().gap_2().child(label(sc_core::t!("tools.field.name"))).child(div().flex_1().child(Input::new(&self.name).disabled(none))))
            .child(h_flex().gap_2().child(label(sc_core::t!("tools.field.command"))).child(div().flex_1().child(Input::new(&self.command).disabled(none))))
            .child(
                h_flex().gap_2().child(label(String::new())).child(
                    Checkbox::new("tool-terminal")
                        .label(sc_core::t!("tools.run_in_terminal"))
                        .checked(tool.terminal)
                        .disabled(none)
                        .on_click(cx.listener(|this, checked: &bool, window, cx| this.edit(window, cx, |tool| tool.terminal = *checked))),
                ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(label(sc_core::t!("tools.field.key")))
                    .child(div().min_w(px(160.)).child(key))
                    .child(Button::new("tool-key").outline().label(sc_core::t!("tools.set_key")).disabled(none).on_click(cx.listener(|this, _, _, cx| {
                        this.capturing = true;
                        cx.notify();
                    })))
                    .child(Button::new("tool-key-clear").outline().label(sc_core::t!("tools.clear_key")).disabled(none || tool.key.is_empty()).on_click(cx.listener(|this, _, window, cx| {
                        this.note = None;
                        this.edit(window, cx, |tool| tool.key.clear());
                    }))),
            )
            .children(self.note.clone().map(|note| div().text_color(theme.warning).child(note)))
            .child(div().text_xs().text_color(theme.muted_foreground).child(sc_core::t!("tools.placeholders")));
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(div().text_color(theme.muted_foreground).child(sc_core::t!("tools.intro")))
            .child(list)
            .child(buttons)
            .child(editor)
            .child(h_flex().child(div().flex_1()).child(Button::new("tools-close").primary().label(sc_core::t!("tools.close")).min_w(px(80.)).on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx))))
    }
}
