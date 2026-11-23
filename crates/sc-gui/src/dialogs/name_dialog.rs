use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::v_flex;
use gpui_kit::*;

type Done = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// Asks for a name; `done` gets it on OK or Enter.
pub fn open(
    title: impl Into<SharedString>,
    label: impl Into<SharedString>,
    value: String,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(String, &mut Window, &mut App) + 'static,
) {
    let (title, label, done): (SharedString, _, Done) = (title.into(), label.into(), Rc::new(done));
    let build = move |main, window: &mut Window, cx: &mut App| {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
        crate::dialogs::pack_dialog::focus_all(&input, window, cx);
        cx.new(|_| NameDialog { label, input, done, main })
    };
    let ok = |view: &Entity<NameDialog>, window: &mut Window, cx: &mut App| {
        view.update(cx, |this, cx| this.finish(window, cx));
        true
    };
    crate::dialogs::dialog_window::show(title.to_string(), size(px(400.), px(130.)), window, cx, build, ok);
}

struct NameDialog {
    label: SharedString,
    input: Entity<InputState>,
    done: Done,
    main: Option<crate::dialogs::dialog_window::Main>,
}

impl NameDialog {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (name, done) = (self.input.read(cx).value().to_string(), self.done.clone());
        crate::dialogs::dialog_window::answer(self.main, window, cx, move |window, cx| done(name, window, cx));
    }
}

impl Render for NameDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                v_flex()
                    .gap_1()
                    .child(self.label.clone())
                    .child(Input::new(&self.input)),
            )
            .child(
                crate::dialogs::footer(cx)
                    .children(crate::dialogs::ok_cancel(
                        Button::new("name-ok")
                            .primary()
                            .label(sc_core::t!("name_dialog.ok"))
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish(window, cx);
                                crate::dialogs::dialog_window::close(window, cx);
                            })),
                        Button::new("name-cancel")
                            .outline()
                            .label(sc_core::t!("name_dialog.cancel"))
                            .min_w(px(80.))
                            .on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx)),
                    )),
            )
    }
}
