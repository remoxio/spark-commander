use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, IconName, h_flex, v_flex};
use gpui_kit::*;
use sc_core::t;
use sc_core::viewer::Query;

type Done = Rc<dyn Fn(Query, &mut Window, &mut App)>;

/// Lister's Find dialog; `done` gets the query on OK or Enter.
pub fn open(
    query: Query,
    history: Vec<String>,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(Query, &mut Window, &mut App) + 'static,
) {
    let done: Done = Rc::new(done);
    let build = move |main, window: &mut Window, cx: &mut App| {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(query.text.clone()));
        crate::dialogs::pack_dialog::focus_all(&input, window, cx);
        cx.new(|_| FindDialog { input, query, history, done, main })
    };
    let ok = |view: &Entity<FindDialog>, window: &mut Window, cx: &mut App| {
        view.update(cx, |this, cx| this.finish(window, cx));
        true
    };
    crate::dialogs::dialog_window::show(t!("find.title"), size(px(460.), px(260.)), window, cx, build, ok);
}

struct FindDialog {
    input: Entity<InputState>,
    /// Its options; the text is the input's.
    query: Query,
    history: Vec<String>,
    done: Done,
    main: Option<crate::dialogs::dialog_window::Main>,
}

impl FindDialog {
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = Query {
            text: self.input.read(cx).value().to_string(),
            ..self.query.clone()
        };
        let done = self.done.clone();
        crate::dialogs::dialog_window::answer(self.main, window, cx, move |window, cx| done(query, window, cx));
    }

    fn option(
        &self,
        id: &'static str,
        label: String,
        on: fn(&mut Query) -> &mut bool,
        cx: &mut Context<Self>,
    ) -> Checkbox {
        let mut query = self.query.clone();
        Checkbox::new(id)
            .label(label)
            .checked(*on(&mut query))
            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                *on(&mut this.query) = *checked;
                cx.notify();
            }))
    }
}

impl Render for FindDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input = self.input.clone();
        let history = self.history.clone();
        let earlier = Button::new("find-history")
            .ghost()
            .icon(IconName::ChevronDown)
            .disabled(history.is_empty())
            .dropdown_menu(move |mut menu, _, _| {
                for text in &history {
                    let (input, text) = (input.clone(), text.clone());
                    let item = PopupMenuItem::new(text.clone()).on_click(move |_, window, cx| {
                        input.update(cx, |input, cx| input.set_value(text.clone(), window, cx))
                    });
                    menu = menu.item(item);
                }
                menu
            });
        let left = v_flex()
            .flex_1()
            .gap_2()
            .child(self.option("find-words", t!("find.whole_words"), |q| &mut q.whole_words, cx))
            .child(self.option("find-back", t!("find.backwards"), |q| &mut q.backwards, cx))
            .child(self.option("find-regex", t!("find.regex"), |q| &mut q.regex, cx));
        let right = v_flex()
            .flex_1()
            .gap_2()
            .child(self.option("find-case", t!("find.case_sensitive"), |q| &mut q.case_sensitive, cx))
            .child(self.option("find-hex", t!("find.hex"), |q| &mut q.hex, cx));
        v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                v_flex().gap_1().child(t!("find.text.label")).child(
                    h_flex()
                        .gap_1()
                        .child(div().flex_1().child(Input::new(&self.input)))
                        .child(earlier),
                ),
            )
            .child(h_flex().items_start().child(left).child(right))
            .child(
                crate::dialogs::footer(cx)
                    .children(crate::dialogs::ok_cancel(
                        Button::new("find-ok")
                            .primary()
                            .label(t!("find.ok"))
                            .min_w(px(80.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish(window, cx);
                                crate::dialogs::dialog_window::close(window, cx);
                            })),
                        Button::new("find-cancel")
                            .outline()
                            .label(t!("find.cancel"))
                            .min_w(px(80.))
                            .on_click(|_, window, cx| crate::dialogs::dialog_window::close(window, cx)),
                    )),
            )
    }
}
