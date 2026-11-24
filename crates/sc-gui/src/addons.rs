//! The add-ons' downloads and updates (sc_core::addons) for Settings >
//! Add-ons: the list looked up at start and daily, updates offered in a
//! notification, installs run in the background with their progress; a
//! missing one offered when a file needs it.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{WindowExt, v_flex};
use gpui_kit::*;
use sc_core::addons::{Addon, Manifest, Presence};
use sc_core::t;

use crate::dialogs::Role;

/// After the window is up, then a day apart.
const FIRST: Duration = Duration::from_secs(5);
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Default)]
pub struct Addons {
    pub manifest: Option<Manifest>,
    pub checking: bool,
    /// Why the last look-up failed.
    pub check_error: Option<String>,
    /// Each add-on as last looked at.
    pub presence: HashMap<Addon, Presence>,
    /// Downloads under way: the bytes so far, of.
    pub progress: HashMap<Addon, (u64, u64)>,
    /// Why an add-on's last install or removal failed.
    pub failed: HashMap<Addon, String>,
}

impl Addons {
    fn refresh(&mut self) {
        self.presence = Addon::ALL.into_iter().map(|addon| (addon, addon.presence())).collect();
    }
}

struct AddonsGlobal(Entity<Addons>);

impl Global for AddonsGlobal {}

pub fn entity(cx: &App) -> Entity<Addons> {
    cx.global::<AddonsGlobal>().0.clone()
}

/// The add-ons as they are now, and the look-ups at start and daily.
pub fn init(cx: &mut App) {
    let addons = cx.new(|_| {
        let mut addons = Addons::default();
        addons.refresh();
        addons
    });
    cx.set_global(AddonsGlobal(addons));
    cx.spawn(async move |cx| {
        let mut wait = FIRST;
        loop {
            cx.background_executor().timer(wait).await;
            wait = DAY;
            cx.update(|cx| {
                if crate::current_settings(cx).is_some_and(|settings| settings.addon_updates.0) {
                    check(true, cx);
                }
            });
        }
    })
    .detach();
}

/// Settings > Add-ons is shown: each add-on looked at again, the list looked up
/// when not yet.
pub fn shown(cx: &mut App) {
    let addons = entity(cx);
    let unknown = addons.update(cx, |this, cx| {
        this.refresh();
        cx.notify();
        this.manifest.is_none()
    });
    if unknown {
        check(false, cx);
    }
}

/// Looks up the add-ons' list; with `offer`, a notification offers the updates.
pub fn check(offer: bool, cx: &mut App) {
    let addons = entity(cx);
    if addons.read(cx).checking {
        return;
    }
    addons.update(cx, |this, cx| {
        this.checking = true;
        cx.notify();
    });
    let fetched = cx.background_spawn(async { sc_core::addons::fetch() });
    cx.spawn(async move |cx| {
        let fetched = fetched.await;
        cx.update(|cx| {
            let updates = addons.update(cx, |this, cx| {
                this.checking = false;
                this.refresh();
                cx.notify();
                match fetched {
                    Ok(manifest) => {
                        this.check_error = None;
                        let updates = manifest.updates();
                        this.manifest = Some(manifest);
                        updates
                    }
                    Err(err) => {
                        this.check_error = Some(err.to_string());
                        Vec::new()
                    }
                }
            });
            if offer && !updates.is_empty() {
                offer_updates(updates, cx);
            }
        });
    })
    .detach();
}

fn offer_updates(updates: Vec<(Addon, String)>, cx: &mut App) {
    let names: Vec<String> = updates.iter().map(|(addon, version)| format!("{} {version}", addon.name())).collect();
    let text = t!("addons.updates", names = names.join(", "));
    let addons: Vec<Addon> = updates.into_iter().map(|(addon, _)| addon).collect();
    crate::notify_main(
        Notification::info(text).action(move |_, _, _| {
            let addons = addons.clone();
            Button::new("addon-update").primary().label(t!("addons.update")).on_click(move |_, _, cx| {
                for &addon in &addons {
                    install(addon, cx);
                }
            })
        }),
        cx,
    );
}

/// `name` needs the missing `addon`: Yes shows Settings > Add-ons; No, and
/// Never show again (which the settings keep), `go_on` without it.
pub fn offer(
    addon: Addon,
    name: &str,
    commander: WeakEntity<crate::Commander>,
    window: &mut Window,
    cx: &mut App,
    go_on: impl Fn(&mut Window, &mut App) + 'static,
) {
    let text = t!("addons.offer.text", name = name, addon = addon.name());
    let go_on: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(go_on);
    if crate::dialogs::dialog_window::separate() {
        let build = move |main, _: &mut Window, cx: &mut App| {
            let view = cx.new(|_| Offer { text, commander, go_on, main, answered: false });
            // Closed unanswered (Escape): as Cancel, it goes on without.
            view.update(cx, |_, cx| {
                cx.on_release(|this: &mut Offer, cx| {
                    if !this.answered {
                        let go_on = this.go_on.clone();
                        this.main.run(cx, move |window, cx| go_on(window, cx));
                    }
                })
                .detach()
            });
            (view, None)
        };
        return crate::dialogs::dialog_window::open(t!("addons.offer.title"), size(px(480.), px(170.)), window, cx, build);
    }
    window.open_dialog(cx, move |dialog, _, cx| {
        let (yes, no, never, cancel) = (commander.clone(), go_on.clone(), go_on.clone(), go_on.clone());
        let commander = commander.clone();
        dialog
            .title(t!("addons.offer.title"))
            .width(px(480.))
            .overlay_closable(false)
            .on_cancel(move |_, window, cx| {
                cancel(window, cx);
                true
            })
            .child(
                v_flex()
                    .gap_3()
                    .text_sm()
                    .child(div().whitespace_normal().child(text.clone()))
                    .child(
                        crate::dialogs::footer(cx).children(crate::dialogs::ordered([
                            (Role::Main, Button::new("offer-yes").primary().label(t!("addons.offer.yes")).on_click(move |_, window, cx| {
                                crate::dialogs::dialog_window::close(window, cx);
                                let Some(settings) = yes.upgrade().map(|this| this.read(cx).settings.clone()) else { return };
                                let page = sc_core::settings::PAGES.iter().position(|&page| page == "Add-ons").unwrap_or(0);
                                crate::dialogs::settings_dialog::open(yes.clone(), settings, page, window, cx);
                            })),
                            (Role::Dismiss, Button::new("offer-no").outline().label(t!("addons.offer.no")).on_click(move |_, window, cx| {
                                crate::dialogs::dialog_window::close(window, cx);
                                no(window, cx);
                            })),
                            (Role::Other, Button::new("offer-never").outline().label(t!("addons.offer.never")).on_click(
                                move |_, window, cx| {
                                    crate::dialogs::dialog_window::close(window, cx);
                                    let _ = commander.update(cx, |this, cx| {
                                        this.settings.never_offer_addons = true;
                                        this.save_settings(window, cx);
                                    });
                                    never(window, cx);
                                },
                            )),
                        ])),
                    ),
            )
    });
}

/// Downloads and installs the add-on's current release.
pub fn install(addon: Addon, cx: &mut App) {
    let addons = entity(cx);
    let state = addons.read(cx);
    if state.progress.contains_key(&addon) {
        return;
    }
    let Some((release, download)) = state.manifest.as_ref().and_then(|manifest| manifest.offer(addon)) else {
        return;
    };
    let (release, download) = (release.clone(), download.clone());
    addons.update(cx, |this, cx| {
        this.progress.insert(addon, (0, download.size));
        this.failed.remove(&addon);
        cx.notify();
    });
    let done = Arc::new(AtomicU64::new(0));
    let version = release.version.clone();
    let installed = cx.background_spawn({
        let done = done.clone();
        async move { sc_core::addons::install(addon, &release, &download, &|bytes| done.store(bytes, Ordering::Relaxed)) }
    });
    // The bytes so far, while it lasts.
    cx.spawn({
        let addons = addons.clone();
        async move |cx| loop {
            cx.background_executor().timer(Duration::from_millis(150)).await;
            let going = cx.update(|cx| {
                addons.update(cx, |this, cx| match this.progress.get_mut(&addon) {
                    Some(progress) => {
                        progress.0 = done.load(Ordering::Relaxed);
                        cx.notify();
                        true
                    }
                    None => false,
                })
            });
            if !going {
                break;
            }
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        let installed = installed.await;
        cx.update(|cx| {
            addons.update(cx, |this, cx| {
                this.progress.remove(&addon);
                if let Err(err) = &installed {
                    this.failed.insert(addon, err.to_string());
                }
                this.refresh();
                cx.notify();
            });
            let notification = match installed {
                Ok(()) => Notification::success(t!("addons.installed", addon = addon.name(), version = version)),
                Err(err) => Notification::error(format!("{}: {err}", addon.name())),
            };
            crate::notify_main(notification, cx);
        });
    })
    .detach();
}

/// Removes the installed add-on.
pub fn remove(addon: Addon, cx: &mut App) {
    let addons = entity(cx);
    let removed = cx.background_spawn(async move { sc_core::addons::remove(addon) });
    cx.spawn(async move |cx| {
        let removed = removed.await;
        cx.update(|cx| {
            addons.update(cx, |this, cx| {
                match removed {
                    Ok(()) => this.failed.remove(&addon),
                    Err(err) => this.failed.insert(addon, err.to_string()),
                };
                this.refresh();
                cx.notify();
            })
        });
    })
    .detach();
}

/// "24.3 MB".
pub fn megabytes(bytes: u64) -> String {
    t!("addons.megabytes", size = format!("{:.1}", bytes as f64 / 1_000_000.))
}

/// The offer in a dialog window: its answer goes to the main window.
struct Offer {
    text: String,
    commander: WeakEntity<crate::Commander>,
    go_on: Rc<dyn Fn(&mut Window, &mut App)>,
    main: crate::dialogs::dialog_window::Main,
    answered: bool,
}

impl Offer {
    fn answer(&mut self, window: &mut Window, cx: &mut App, then: impl FnOnce(&mut Window, &mut App) + 'static) {
        self.answered = true;
        crate::dialogs::dialog_window::close(window, cx);
        self.main.run(cx, then);
    }
}

impl Render for Offer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .text_sm()
            .child(div().whitespace_normal().child(self.text.clone()))
            .child(
                crate::dialogs::responses(cx, [
                    (Role::Main, Button::new("offer-yes").primary().label(t!("addons.offer.yes")).on_click(cx.listener(|this, _, window, cx| {
                        let commander = this.commander.clone();
                        this.answer(window, cx, move |window, cx| {
                            let Some(settings) = commander.upgrade().map(|this| this.read(cx).settings.clone()) else { return };
                            let page = sc_core::settings::PAGES.iter().position(|&page| page == "Add-ons").unwrap_or(0);
                            crate::dialogs::settings_dialog::open(commander, settings, page, window, cx);
                        });
                    }))),
                    (Role::Dismiss, Button::new("offer-no").outline().label(t!("addons.offer.no")).on_click(cx.listener(|this, _, window, cx| {
                        let go_on = this.go_on.clone();
                        this.answer(window, cx, move |window, cx| go_on(window, cx));
                    }))),
                    (Role::Other, Button::new("offer-never").outline().label(t!("addons.offer.never")).on_click(cx.listener(|this, _, window, cx| {
                        let (commander, go_on) = (this.commander.clone(), this.go_on.clone());
                        this.answer(window, cx, move |window, cx| {
                            let _ = commander.update(cx, |this, cx| {
                                this.settings.never_offer_addons = true;
                                this.save_settings(window, cx);
                            });
                            go_on(window, cx);
                        });
                    }))),
                ]),
            )
    }
}
