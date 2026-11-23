//! Associate With and Internal Associations: by file mask, the programs
//! Enter, F3 and F4 run. Kept in the settings.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use sc_core::associations::{self, Association};

use crate::Commander;

type Saved = Rc<dyn Fn(Association, &mut Window, &mut App)>;

impl Commander {
    /// The cursor's file type: its association, new or to change.
    pub(crate) fn associate_with(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panels[self.active].tabs.current();
        let Some(entry) = panel.current_entry().filter(|e| !e.is_dir) else {
            let text = sc_core::t!("associations.no_file");
            return window.push_notification(Notification::warning(text), cx);
        };
        let mask = associations::mask_for(&entry.display_name());
        let list = &self.settings.associations;
        let at = list.iter().position(|a| a.mask.eq_ignore_ascii_case(&mask));
        let association = match at {
            Some(at) => list[at].clone(),
            None => Association {
                mask,
                ..Association::default()
            },
        };
        let commander = cx.entity().downgrade();
        edit(association, window, cx, move |association, window, cx| {
            let _ = commander.update(cx, |this, cx| {
                let mut list = this.settings.associations.clone();
                match at {
                    Some(at) => list[at] = association,
                    None => list.push(association),
                }
                this.save_associations(list, window, cx);
            });
        });
    }

    pub(crate) fn internal_associations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commander = cx.entity().downgrade();
        let build = move |_, _: &mut Window, cx: &mut App| cx.new(|_| AssociationsDialog { commander });
        crate::dialogs::dialog_window::show(sc_core::t!("associations.title"), size(px(820.), px(440.)), window, cx, build, |_, _, _| false);
    }

    fn save_associations(
        &mut self,
        list: Vec<Association>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.associations = list;
        if let Err(err) = self.settings.save() {
            let text = sc_core::t!("associations.save_failed", err = err);
            window.push_notification(Notification::error(text), cx);
        }
        cx.notify();
    }
}

struct AssociationsDialog {
    commander: WeakEntity<Commander>,
}

impl AssociationsDialog {
    fn change(
        &self,
        cx: &mut App,
        change: impl FnOnce(&mut Vec<Association>),
        window: &mut Window,
    ) {
        let _ = self.commander.update(cx, |this, cx| {
            let mut list = this.settings.associations.clone();
            change(&mut list);
            this.save_associations(list, window, cx);
        });
    }

    fn open_edit(&self, at: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let list = self
            .commander
            .read_with(cx, |this, _| this.settings.associations.clone())
            .unwrap_or_default();
        let association = at.and_then(|at| list.get(at).cloned()).unwrap_or_default();
        let this = cx.entity().downgrade();
        edit(association, window, cx, move |association, window, cx| {
            let _ = this.update(cx, |this, cx| {
                this.change(
                    cx,
                    |list| match at {
                        Some(at) if at < list.len() => list[at] = association,
                        _ => list.push(association),
                    },
                    window,
                );
                cx.notify();
            });
        });
    }
}

impl Render for AssociationsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self
            .commander
            .read_with(cx, |this, _| this.settings.associations.clone())
            .unwrap_or_default();
        let muted = cx.theme().muted_foreground;
        let cell = |text: &str, width: f32| {
            div()
                .w(px(width))
                .flex_none()
                .truncate()
                .child(if text.is_empty() {
                    "–".to_string()
                } else {
                    text.to_string()
                })
        };
        let rows = list.iter().enumerate().map(|(ix, a)| {
            h_flex()
                .gap_2()
                .child(cell(&a.mask, 150.))
                .child(cell(&a.open, 160.))
                .child(cell(&a.view, 160.))
                .child(cell(&a.edit, 160.))
                .child(
                    Button::new(("assoc-edit", ix))
                        .xsmall()
                        .outline()
                        .label(sc_core::t!("associations.edit"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_edit(Some(ix), window, cx)
                        })),
                )
                .child(
                    Button::new(("assoc-remove", ix))
                        .xsmall()
                        .outline()
                        .label(sc_core::t!("associations.remove"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.change(cx, |list| drop(list.remove(ix)), window);
                            cx.notify();
                        })),
                )
        });
        v_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .child(sc_core::t!("associations.intro"))
            .child(
                h_flex()
                    .gap_2()
                    .text_color(muted)
                    .child(cell(&sc_core::t!("associations.column.files"), 150.))
                    .child(cell(&sc_core::t!("associations.column.open"), 160.))
                    .child(cell(&sc_core::t!("associations.column.view"), 160.))
                    .child(cell(&sc_core::t!("associations.column.edit"), 160.)),
            )
            .children(rows)
            .when(list.is_empty(), |el| el.child(div().text_color(muted).child(sc_core::t!("associations.empty"))))
            .child(
                crate::dialogs::footer(cx)
                    .child(
                        Button::new("assoc-add")
                            .outline()
                            .label(sc_core::t!("associations.add"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_edit(None, window, cx))),
                    )
                    .child(
                        Button::new("assoc-close")
                            .primary()
                            .label(sc_core::t!("associations.close"))
                            .min_w(px(80.))
                            .on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx)),
                    ),
            )
    }
}

/// One association's mask and programs; `done` gets it on OK.
fn edit(
    association: Association,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(Association, &mut Window, &mut App) + 'static,
) {
    let done: Saved = Rc::new(done);
    let build = move |main, window: &mut Window, cx: &mut App| {
    let fields = [
        &association.mask,
        &association.open,
        &association.view,
        &association.edit,
    ]
    .map(|value| {
        let value = value.clone();
        cx.new(|cx| InputState::new(window, cx).default_value(value))
    });
    crate::dialogs::pack_dialog::focus_all(&fields[0], window, cx);
    cx.new(|_| EditAssociation { fields, done, main })
    };
    let ok = |view: &Entity<EditAssociation>, window: &mut Window, cx: &mut App| view.update(cx, |this, cx| this.finish(window, cx));
    crate::dialogs::dialog_window::show(sc_core::t!("associations.edit_dialog.title"), size(px(560.), px(260.)), window, cx, build, ok);
}

struct EditAssociation {
    /// Mask, open, view, edit.
    fields: [Entity<InputState>; 4],
    done: Saved,
    main: Option<crate::dialogs::dialog_window::Main>,
}

impl EditAssociation {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let [mask, open, view, edit] = self
            .fields
            .each_ref()
            .map(|input| input.read(cx).value().trim().to_string());
        if mask.is_empty() {
            window.push_notification(Notification::error(sc_core::t!("associations.mask_needed")), cx);
            return false;
        }
        let done = self.done.clone();
        crate::dialogs::dialog_window::answer(self.main, window, cx, move |window, cx| done(Association { mask, open, view, edit }, window, cx));
        true
    }
}

impl Render for EditAssociation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let labels = [
            sc_core::t!("associations.field.files"),
            sc_core::t!("associations.field.open"),
            sc_core::t!("associations.field.view"),
            sc_core::t!("associations.field.edit"),
        ];
        v_flex()
            .w_full()
            .gap_2()
            .text_sm()
            .children(labels.iter().zip(&self.fields).map(|(label, input)| {
                h_flex()
                    .gap_2()
                    .child(div().w(px(150.)).flex_none().child(label.clone()))
                    .child(div().flex_1().child(Input::new(input).small()))
            }))
            .child(
                crate::dialogs::footer(cx)
                    .child(
                        Button::new("assoc-ok")
                            .primary()
                            .label(sc_core::t!("associations.ok"))
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.finish(window, cx) {
                                    crate::dialogs::dialog_window::close(window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("assoc-cancel")
                            .outline()
                            .label(sc_core::t!("associations.cancel"))
                            .min_w(px(80.))
                            .on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx)),
                    ),
            )
    }
}
