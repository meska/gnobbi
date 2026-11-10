//! Removing one account from the drawer, after an explicit confirmation: its worker revokes
//! the session and stops, then its vault items, folders and catalog entry go. No other
//! account is touched. The window moves to another account, or starts a sign-in when none is left.
use crate::*;

pub(crate) struct Removal {
    pub(crate) id: AccountId,
    /// The account was the only one the window could show: a sign-in replaces this window.
    leave: bool,
    result: std::sync::mpsc::Receiver<bool>,
}

#[cfg(feature = "live")]
fn forget(id: AccountId, stores: &[std::path::PathBuf]) -> bool {
    matrix_engine::signin::forget_secrets(id, stores).is_ok()
}
#[cfg(not(feature = "live"))]
fn forget(_: AccountId, _: &[std::path::PathBuf]) -> bool {
    true
}

/// Blocking; false when the account could not be fully removed (it then stays in the catalog).
fn erase(
    dirs: &matrix_desktop::AppDirs,
    id: AccountId,
    control: Option<matrix_engine::WorkerControl>,
) -> bool {
    if let Some(control) = control {
        control.logout_and_stop();
        let start = std::time::Instant::now();
        while !control.is_done() {
            if start.elapsed() > std::time::Duration::from_secs(60) {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    // Another window may hold this profile: its files are not ours to delete.
    #[cfg(feature = "live")]
    let _lock = match matrix_desktop::AccountCatalog::load(dirs)
        .data_dir(dirs, id)
        .map(matrix_engine::signin::SignInProfile::open)
    {
        Some(Err(matrix_engine::signin::SignInError::ProfileBusy)) => return false,
        other => other,
    };
    matrix_desktop::erase_account(dirs, id, |stores| forget(id, stores)).is_ok()
}

impl Shell {
    /// The confirmed account goes away. The shown account hands the window to another one first.
    pub(crate) fn remove_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(id), Some(dirs)) = (self.confirm_removal.take(), self.prefs_dirs.clone()) else {
            return;
        };
        let leave = self.active_id == Some(id) && {
            let next = self.parked.keys().copied().find(|k| self.can_switch_to(*k));
            next.is_none_or(|next| {
                self.switch_account(next, window, cx);
                false
            })
        };
        let control = if leave {
            self.control.take()
        } else {
            self.parked.remove(&id).and_then(|state| state.control)
        };
        let (sender, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(erase(&dirs, id, control));
        });
        self.removals.push(Removal { id, leave, result });
        self.set_menu_open(false, window, cx);
        cx.notify();
    }

    /// Collects finished removals (called on every tick).
    pub(crate) fn finish_removals(&mut self, cx: &mut Context<Self>) {
        let mut quit = false;
        self.removals
            .retain(|removal| match removal.result.try_recv() {
                Err(std::sync::mpsc::TryRecvError::Empty) => true,
                done => {
                    let ok = done == Ok(true);
                    if !ok {
                        eprintln!("Gnobbi: account removal did not finish");
                    }
                    if ok {
                        // Room names, ids and avatars of the account must not outlive it.
                        self.recents.remove(removal.id.as_u64());
                        self.rail_ui.layouts.remove(removal.id.as_u64());
                        if let Some(dirs) = &self.prefs_dirs {
                            let mut servers = matrix_desktop::directory_servers::SavedServers::load(dirs);
                            servers.remove_account(removal.id.as_u64());
                            let _ = servers.save(dirs);
                            let _ = self.recents.save(dirs);
                            let _ = self.rail_ui.layouts.save(dirs);
                        }
                    }
                    quit |= ok && removal.leave;
                    false
                }
            });
        if let Some(dirs) = &self.prefs_dirs {
            self.accounts = matrix_desktop::AccountCatalog::load(dirs);
        }
        if quit && self.launch_signin() {
            cx.quit();
        }
    }

    pub(crate) fn render_removal_dialog(
        &self,
        t: matrix_desktop::DesignTokens,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let account = self
            .drawer_accounts()
            .into_iter()
            .find(|a| Some(a.id) == self.confirm_removal)?;
        let button = |id: &'static str, label: &'static str, color: u32| {
            div()
                .id(id)
                .px_4()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .text_color(rgb(color))
                .child(label)
        };
        Some(
            div()
                .id("removal-layer")
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .right_0()
                .flex()
                .items_center()
                .justify_center()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_removal = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("removal-dialog")
                        .w(px(320.))
                        .p_4()
                        .gap_2()
                        .flex()
                        .flex_col()
                        .rounded_lg()
                        .bg(rgb(t.surface))
                        .border_1()
                        .border_color(rgb(t.border))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(t.text))
                                .child("Rimuovere l'account?"),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_color(rgb(t.text))
                                .child(account.name.clone()),
                        )
                        .when(account.name != account.user_id, |d| {
                            d.child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .text_color(rgb(t.secondary))
                                    .child(account.user_id.clone()),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(button("removal-cancel", "Annulla", t.accent).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.confirm_removal = None;
                                        cx.notify();
                                    }),
                                ))
                                .child(button("removal-confirm", "Rimuovi", t.error).on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.remove_account(window, cx)
                                    }),
                                )),
                        ),
                )
                .into_any_element(),
        )
    }
}
