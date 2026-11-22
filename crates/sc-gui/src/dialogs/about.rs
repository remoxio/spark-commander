//! Help > About: the app's name, version, site and licence, and the other
//! projects' work in it (`sc_core::about`).

use std::sync::{Arc, OnceLock};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, StyledExt, h_flex, v_flex};
use gpui_kit::*;
use sc_core::about;
use sc_core::addons::{Addon, Builtin};
use sc_core::t;

pub fn open(window: &mut Window, cx: &mut App) {
    crate::dialogs::dialog_window::show(t!("about.title"), size(px(600.), px(560.)), window, cx, |_, _, cx| cx.new(|_| About), |_, _, _| false);
}

struct About;

impl Render for About {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        content(cx)
    }
}

/// The window's icon, as GPUI draws pictures (BGRA); made once.
fn icon() -> Option<Arc<RenderImage>> {
    static ICON: OnceLock<Option<Arc<RenderImage>>> = OnceLock::new();
    ICON.get_or_init(|| {
        let mut pixels = (*crate::window_icon()?).clone();
        pixels.pixels_mut().for_each(|pixel| pixel.0.swap(0, 2));
        Some(Arc::new(RenderImage::new([image::Frame::new(pixels)])))
    })
    .clone()
}

fn link(id: impl Into<ElementId>, label: impl Into<SharedString>, url: &'static str) -> Button {
    Button::new(id).link().label(label).on_click(move |_, _, cx| cx.open_url(url))
}

fn content(cx: &App) -> impl IntoElement + use<> {
    let dim = cx.theme().muted_foreground;
    let site = about::WEBSITE.trim_start_matches("https://");
    let (licence, licence_url) = about::LICENCE;
    let header = h_flex()
        .gap_4()
        .items_start()
        .children(icon().map(|icon| img(ImageSource::Render(icon)).size(px(72.)).flex_none()))
        .child(
            v_flex()
                .gap_1()
                .items_start()
                .child(div().text_xl().font_semibold().child(about::NAME))
                .child(div().text_color(dim).child(t!("about.version", version = about::VERSION)))
                .child(t!("about.tagline"))
                .child(link("about-site", site, about::WEBSITE))
                .child(h_flex().gap_1().child(t!("about.licence")).child(link("about-licence", licence, licence_url))),
        );
    // Each project: its name to its site, its licence, what it does here.
    let row = |group: &'static str, ix: usize, name: &str, what: &str, licence: &str, website: &'static str| {
        v_flex()
            .py_0p5()
            .child(
                h_flex()
                    .gap_2()
                    .child(link((group, ix), name.to_owned(), website))
                    .child(div().text_xs().text_color(dim).child(licence.to_owned())),
            )
            .child(div().text_xs().text_color(dim).child(what.to_owned()))
    };
    let heading = |text: String| div().pt_2().font_semibold().child(text);
    let shipped = Builtin::ALL
        .iter()
        .map(|b| (b.name(), b.purpose(), b.licence(), b.website().1))
        .chain(about::SHIPPED.iter().map(|p| (p.name, (p.what)(), p.licence, p.website)));
    let addons = Addon::ALL.iter().map(|a| (a.name(), a.purpose(), a.licence(), a.website().1));
    let libraries = about::LIBRARIES.iter().map(|p| (p.name, (p.what)(), p.licence, p.website));
    let mut parts = v_flex().pr_3();
    for (group, title, list) in [
        ("shipped", t!("about.shipped"), shipped.collect::<Vec<_>>()),
        ("addons", t!("about.addons"), addons.collect()),
        ("libraries", t!("about.libraries"), libraries.collect()),
    ] {
        parts = parts.child(heading(title));
        for (ix, (name, what, licence, website)) in list.into_iter().enumerate() {
            parts = parts.child(row(group, ix, name, &what, licence, website));
        }
    }
    v_flex()
        .gap_3()
        .text_sm()
        .child(header)
        .child(div().border_t_1().border_color(cx.theme().border))
        .child(div().font_semibold().child(t!("about.made_with")))
        // A box of its own height, the list scrolling in it.
        .child(v_flex().h(px(340.)).child(div().id("about-parts").flex_1().min_h_0().overflow_y_scrollbar().child(parts)))
}
