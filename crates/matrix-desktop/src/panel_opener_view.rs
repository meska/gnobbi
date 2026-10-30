//! Focus and callbacks for the existing root panel buttons.
use crate::*;
use matrix_desktop::panel_openers::{Control, Intent};

pub(crate) struct OpenerUi {
    sources: [Option<u64>; 3],
    next: u64,
    context: Option<(AccountId, Option<RoomScope>)>,
    focus: std::cell::RefCell<[Option<(u64, FocusHandle)>; 3]>,
    bounds: [Option<(u64, Bounds<gpui::Pixels>, f32)>; 3],
}
impl OpenerUi {
    pub(crate) fn new() -> Self {
        Self {
            sources: [None; 3],
            next: 1,
            context: None,
            bounds: [None; 3],
            focus: Default::default(),
        }
    }
    fn focus_handle(&self, control: Control, cx: &mut App) -> Option<FocusHandle> {
        let source = self.source(control)?;
        let mut handles = self.focus.borrow_mut();
        let slot = &mut handles[control as usize];
        if !slot.as_ref().is_some_and(|(current, _)| *current == source) {
            *slot = Some((source, cx.focus_handle()));
        }
        slot.as_ref().map(|(_, handle)| handle.clone())
    }
    pub(crate) fn painted_bounds(&self, control: Control) -> Option<(Bounds<gpui::Pixels>, f32)> {
        let (source, bounds, scale) = self.bounds[control as usize]?;
        (self.source(control) == Some(source)).then_some((bounds, scale))
    }
    pub(crate) fn source(&self, control: Control) -> Option<u64> {
        self.sources[control as usize]
    }
    /// A hidden opener keeps its old handle; true while that handle still holds native focus.
    pub(crate) fn hidden_focused(&self, window: &Window) -> bool {
        self.focus.borrow().iter().enumerate().any(|(i, slot)| {
            slot.as_ref().is_some_and(|(source, handle)| {
                self.sources[i] != Some(*source) && handle.is_focused(window)
            })
        })
    }
    pub(crate) fn focused(&self, control: Control, window: &Window) -> bool {
        self.focus.borrow()[control as usize]
            .as_ref()
            .is_some_and(|(source, handle)| {
                self.source(control) == Some(*source) && handle.is_focused(window)
            })
    }
}
impl Shell {
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_openers(
        &self,
        window: &Window,
    ) -> Vec<matrix_desktop::panel_openers::Snapshot> {
        Control::ALL
            .into_iter()
            .filter_map(|control| {
                Some(matrix_desktop::panel_openers::Snapshot {
                    control,
                    source: self.opener_ui.source(control)?,
                    focused: self.opener_ui.focused(control, window),
                    bounds: self
                        .opener_ui
                        .painted_bounds(control)
                        .map(|(bounds, scale)| {
                            let scale = f64::from(scale);
                            accesskit::Rect::new(
                                f64::from(f32::from(bounds.left())) * scale,
                                f64::from(f32::from(bounds.top())) * scale,
                                f64::from(f32::from(bounds.right())) * scale,
                                f64::from(f32::from(bounds.bottom())) * scale,
                            )
                        }),
                })
            })
            .collect()
    }
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn dispatch_opener_action(
        &mut self,
        action: &matrix_desktop::accessibility::NavigationAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let matrix_desktop::accessibility::NavigationAction::PanelOpener(intent) = action else {
            return false;
        };
        self.apply_opener(*intent, window, cx);
        true
    }

    pub(crate) fn refresh_openers(&mut self) {
        let context = (
            self.active_scope.account_id,
            self.sidebar.selected().map(|r| r.scope.clone()),
        );
        if self.opener_ui.context.as_ref() != Some(&context) {
            self.opener_ui.sources = [None; 3];
            self.opener_ui.context = Some(context);
        }
        let available = self.connected
            && !self.notifications_open
            && !self.verification_open
            && !self.room_details_open;
        #[cfg(feature = "live")]
        let available = available && self.signin.is_none();
        let visible = [
            available
                && (self.navigation_wide || self.compact_chat)
                && self.sidebar.selected().is_some(),
            false, // Devices and Notifications are drawer rows now
            false,
        ];
        for (i, visible) in visible.into_iter().enumerate() {
            if !visible {
                self.opener_ui.sources[i] = None;
            } else if self.opener_ui.sources[i].is_none() {
                self.opener_ui.sources[i] = Some(self.opener_ui.next);
                self.opener_ui.next = self
                    .opener_ui
                    .next
                    .checked_add(1)
                    .expect("opener source exhausted");
            }
        }
    }
    pub(crate) fn apply_opener(
        &mut self,
        intent: Intent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_navigation_visibility();
        if self.opener_ui.source(intent.control) != Some(intent.source) {
            return;
        }
        if intent.focus_only {
            if let Some(handle) = self.opener_ui.focus_handle(intent.control, cx) {
                handle.focus(window);
            }
        } else {
            match intent.control {
                Control::Info => {
                    self.set_room_details_open(true);
                    self.list_focus.focus(window);
                }
                Control::Notifications => self.open_notifications_panel(window),
                Control::Devices => self.open_devices_panel(window),
            }
        }
        cx.notify();
    }
    pub(crate) fn open_notifications_panel(&mut self, window: &mut Window) {
        self.set_notifications_open(true);
        self.list_focus.focus(window);
    }
    pub(crate) fn open_devices_panel(&mut self, window: &mut Window) {
        self.set_verification_open(true);
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            self.device_focus = Default::default();
            self.a11y_device_bounds = [None; 6];
        }
        self.verification.refresh();
        self.list_focus.focus(window);
    }
    pub(crate) fn opener_callback(
        &self,
        intent: Intent,
    ) -> impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + use<> {
        move |this, window, cx| this.apply_opener(intent, window, cx)
    }
    pub(crate) fn render_root_opener(
        &self,
        control: Control,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(source) = self.opener_ui.source(control) else {
            return div().into_any_element();
        };
        let t = self.theme.tokens();
        let callback = self.opener_callback(Intent {
            control,
            source,
            focus_only: false,
        });
        let focus = self
            .opener_ui
            .focus_handle(control, cx)
            .expect("visible opener");
        let entity = cx.entity();
        let observer = gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                let scale = window.scale_factor();
                if entity.read(cx).opener_ui.painted_bounds(control) != Some((bounds, scale)) {
                    let entity = entity.clone();
                    window.on_next_frame(move |_, cx| {
                        entity.update(cx, |this, cx| {
                            if publish_opener_bounds(control, source, bounds, scale)(this) {
                                cx.notify();
                            }
                        })
                    });
                }
            },
        )
        .absolute()
        .inset_0();
        div()
            .id(match control {
                Control::Info => "room-info",
                Control::Devices => "open-devices",
                Control::Notifications => "open-notifications",
            })
            .relative()
            .child(observer)
            .track_focus(&focus)
            .flex_shrink_0()
            .cursor_pointer()
            .text_color(rgb(t.accent))
            .border_1()
            .border_color(if self.opener_ui.focused(control, window) {
                rgb(t.accent)
            } else {
                gpui::rgba(0)
            })
            .child(match control {
                Control::Info => "Info",
                Control::Devices => "Dispositivi",
                Control::Notifications => "Notifiche",
            })
            .on_click(cx.listener(move |this, _, window, cx| callback(this, window, cx)))
            .into_any_element()
    }
    pub(crate) fn opener_key_callback(
        &self,
    ) -> impl Fn(&mut Shell, &KeyDownEvent, &mut Window, &mut Context<Shell>) -> bool + use<> {
        let sources = self.opener_ui.sources;
        move |this, event, window, cx| {
            let key = &event.keystroke;
            if key.modifiers.control || key.modifiers.alt || key.modifiers.platform || event.is_held
            {
                return false;
            }
            let Some(control) = Control::ALL.into_iter().find(|c| {
                this.opener_ui.focused(*c, window)
                    && sources[*c as usize] == this.opener_ui.source(*c)
            }) else {
                return false;
            };
            if key.key == "tab" {
                this.opener_tab(control, key.modifiers.shift, window, cx);
                return true;
            }
            if !matches!(key.key.as_str(), "space" | "enter") {
                return false;
            }
            if let Some(source) = sources[control as usize] {
                this.apply_opener(
                    Intent {
                        control,
                        source,
                        focus_only: false,
                    },
                    window,
                    cx,
                );
            }
            true
        }
    }
}

fn publish_opener_bounds(
    control: Control,
    source: u64,
    bounds: Bounds<gpui::Pixels>,
    scale: f32,
) -> impl Fn(&mut Shell) -> bool {
    move |this| {
        if this.opener_ui.source(control) != Some(source) {
            return false;
        }
        this.opener_ui.bounds[control as usize] = Some((source, bounds, scale));
        true
    }
}
#[cfg(test)]
pub(crate) use tests::{ACTIVE, install};

#[cfg(test)]
mod tests {
    use crate::*;
    use matrix_desktop::panel_openers::{Control, Intent};
    use std::sync::atomic::{AtomicBool, Ordering};
    pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);
    /// Devices and Notifications moved into the drawer; only Info is still a root button.
    const ROOT: [Control; 1] = [Control::Info];
    static PAINT: AtomicBool = AtomicBool::new(false);
    #[cfg(feature = "live")]
    static AVAILABILITY: AtomicBool = AtomicBool::new(false);

    fn install_paint(window: &mut Window, cx: &mut Context<Shell>) {
        let executor = cx.background_executor().clone();
        cx.spawn_in(window,async move |this,cx| {
            executor.timer(std::time::Duration::from_millis(300)).await;
            this.update_in(cx,|this,window,cx| {
                this.connected=true;this.refresh_navigation_visibility();cx.notify();
                if let Ok(out)=std::env::var("P101_CAPTURE_DIR") {
                    eprintln!("paint probe ready; native activation={}", window.is_window_active());
                    std::fs::write(std::path::Path::new(&out).join("ready"), b"ready").unwrap();
                }
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(500)).await;
            this.update_in(cx,|this,window,cx| {
                assert!(window.is_window_active(), "private runner must activate the ready test window before paint assertions");
                for control in ROOT {
                    assert!(this.opener_ui.painted_bounds(control).is_some(),"current root button must have real painted bounds: {control:?}");
                    #[cfg(all(feature="linux-a11y",target_os="linux"))]
                    assert!(this.semantic_openers(window).iter().find(|s|s.control==control).unwrap().bounds.is_some());
                }
                let control=Control::Info;
                let source=this.opener_ui.source(control).unwrap();
                this.apply_opener(Intent {control,source,focus_only:true},window,cx);
                let (bounds,scale)=this.opener_ui.painted_bounds(control).unwrap();
                let deferred=super::publish_opener_bounds(control,source,bounds,scale);
                assert!(deferred(this));
                this.set_notifications_open(true);this.set_notifications_open(false);
                assert!(!deferred(this),"old deferred paint publication must retire");
                assert!(this.opener_ui.painted_bounds(control).is_none());
                assert!(!this.opener_ui.focused(control,window),"hidden-return must not restore retired Focus");
                eprintln!("real painted bounds/focus/deferred publication retirement passed");cx.notify();
            }).unwrap();
            executor.timer(std::time::Duration::from_millis(400)).await;
            this.update_in(cx,|this,window,cx| {
                for control in ROOT {
                    assert!(this.opener_ui.painted_bounds(control).is_some(),"fresh repaint reacquires bounds");
                    let source=this.opener_ui.source(control).unwrap();
                    this.apply_opener(Intent{control,source,focus_only:true},window,cx);
                    assert!(this.opener_ui.focused(control,window));
                }
            }).unwrap();
            for control in ROOT {
                this.update_in(cx,|this,window,cx| {
                    let source=this.opener_ui.source(control).unwrap();
                    this.apply_opener(Intent{control,source,focus_only:true},window,cx);
                }).unwrap();
                executor.timer(std::time::Duration::from_millis(250)).await;
                if let Ok(out)=std::env::var("P101_CAPTURE_DIR") {
                    let ids=std::process::Command::new("xdotool").args(["search","--onlyvisible","--name","Gnobbi"]).output().unwrap();
                    assert!(ids.status.success());
                    let ids=String::from_utf8(ids.stdout).unwrap();
                    let id=ids.lines().next().unwrap();
                    let file=std::path::Path::new(&out).join(format!("focused-{control:?}.png"));
                    assert!(std::process::Command::new("magick").args(["import","-window",id]).arg(file).status().unwrap().success());
                }
            }
            for chat in [false,true] {
                let captured=this.update_in(cx,|this,window,cx| {
                    use gpui::EntityInputHandler;
                    this.composer.update(cx,|v,cx|v.replace_text_in_range(Some(0..65536)," bozza 🦀\nP101 ",window,cx));
                    navigation_resize_test::FROZEN.store(true,Ordering::SeqCst);
                    this.set_compact_chat(chat);
                    let mut callbacks=vec![];
                    for control in ROOT.into_iter().filter(|_| !chat) {
                        let source=this.opener_ui.source(control).unwrap();
                        let focus=Intent{control,source,focus_only:true};
                        this.apply_opener(focus,window,cx);
                        callbacks.push((this.opener_callback(focus),this.opener_callback(Intent{focus_only:false,..focus}),this.opener_key_callback()));
                    }
                    #[cfg(all(feature="linux-a11y",target_os="linux"))]
                    let intents=semantic_actions(this,window).into_iter().filter(|_| !chat).collect::<Vec<_>>();
                    navigation_resize_test::resize("360","480");
                    (callbacks,#[cfg(all(feature="linux-a11y",target_os="linux"))] intents)
                }).unwrap();
                for width in [360.,1120.] {
                    executor.timer(std::time::Duration::from_millis(450)).await;
                    this.update_in(cx,|this,window,cx| {
                        use gpui::EntityInputHandler;
                        assert_eq!(window.viewport_size().width,px(width));
                        let outside=cx.focus_handle();outside.focus(window);
                        for (focus,click,key) in &captured.0 {
                            focus(this,window,cx);click(this,window,cx);
                            assert!(!key(this,&KeyDownEvent{keystroke:gpui::Keystroke::parse("enter").unwrap(),is_held:false},window,cx));
                        }
                        #[cfg(all(feature="linux-a11y",target_os="linux"))]
                        for intent in &captured.1 {assert!(this.dispatch_opener_action(intent,window,cx));}
                        assert!(outside.is_focused(window));
                        assert!(!this.room_details_open && !this.notifications_open && !this.verification_open);
                        this.composer.update(cx,|v,cx| {let mut actual=None;assert_eq!(v.text_for_range(0..65536,&mut actual,window,cx).as_deref(),Some(" bozza 🦀\nP101 "));});
                        if width==360. {navigation_resize_test::resize("1120","760");}
                    }).unwrap();
                }
            }
            this.update_in(cx,|this,_,cx| {navigation_resize_test::FROZEN.store(false,Ordering::SeqCst);this.set_compact_chat(false);cx.notify();}).unwrap();
            executor.timer(std::time::Duration::from_millis(500)).await;
            this.update_in(cx,|this,window,cx| {
                for control in ROOT {assert!(this.opener_ui.painted_bounds(control).is_some());}
                eprintln!("actual Configure away-back with projection frozen, native callbacks/translated intents rejected, Unicode draft preserved, fresh bounds reacquired");
                let control=Control::Info;let source=this.opener_ui.source(control).unwrap();
                this.opener_callback(Intent{control,source,focus_only:false})(this,window,cx);assert!(this.room_details_open);
                cx.quit();
            }).unwrap();
        }).detach();
    }
    #[test]
    #[ignore = "requires private display"]
    fn painted_opener_bounds_and_focus_retire_with_source() {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        PAINT.store(true, Ordering::SeqCst);
        ACTIVE.store(true, Ordering::SeqCst);
        crate::main();
    }
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    fn semantic_actions(
        this: &Shell,
        window: &Window,
    ) -> Vec<matrix_desktop::accessibility::NavigationAction> {
        use matrix_desktop::accessibility::{
            NavigationSnapshot, semantic_session::SemanticSession,
        };
        let navigation = NavigationSnapshot {
            #[cfg(feature = "live")]
            signin: None,
            openers: this.semantic_openers(window),
            back: None,
            scrollbars: vec![],
            operation_panel: None,
            spaces: None,
            navigation_revision: 0,
            icon_choice: None,
            unread_filter: None,
            new_room: None,
            devices: Default::default(),
            details: Default::default(),
            notifications: Default::default(),
            dark: false,
            visible: true,
            query: String::new(),
            search_focused: false,
            search_bounds: None,
            rooms: vec![],
            history: None,
            list_focused: false,
            composer: None,
        };
        let session = SemanticSession::new(navigation);
        let tree = session.tree();
        let mut result = vec![];
        for control in ROOT {
            let id = tree
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some(control.label()))
                .unwrap()
                .0;
            for action in [accesskit::Action::Focus, accesskit::Action::Click] {
                result.push(
                    session
                        .action(&accesskit::ActionRequest {
                            action,
                            target_tree: accesskit::TreeId::ROOT,
                            target_node: id,
                            data: None,
                        })
                        .unwrap(),
                );
            }
        }
        result
    }
    pub(crate) fn install(window: &mut Window, cx: &mut Context<Shell>) {
        if !ACTIVE.load(Ordering::SeqCst) {
            return;
        }
        #[cfg(feature = "live")]
        if AVAILABILITY.load(Ordering::SeqCst) {
            install_availability(window, cx);
            return;
        }
        if PAINT.load(Ordering::SeqCst) {
            install_paint(window, cx);
            return;
        }
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            executor.timer(std::time::Duration::from_millis(300)).await;
            this.update_in(cx, |this, window, cx| {
                this.connected = true; // deterministic DTO fixture, no worker/network.
                this.refresh_navigation_visibility();
                for control in ROOT {
                    let source = this.opener_ui.source(control).unwrap();
                    this.apply_opener(Intent {control, source, focus_only: true}, window,cx);
                    assert!(this.opener_ui.focused(control, window));
                    assert!(!this.room_details_open && !this.notifications_open && !this.verification_open);
                }
                {
                    let (control,name) = (Control::Info,"space");
                    let source=this.opener_ui.source(control).unwrap();
                    this.apply_opener(Intent {control,source,focus_only:true},window,cx);
                    let key=this.opener_key_callback();
                    assert!(key(this,&KeyDownEvent {keystroke:gpui::Keystroke::parse(name).unwrap(),is_held:false},window,cx));
                    assert_eq!([this.room_details_open,this.verification_open,this.notifications_open],Control::ALL.map(|c| c==control));
                    this.set_room_details_open(false);this.set_verification_open(false);this.set_notifications_open(false);
                }
                for control in ROOT {
                    let source = this.opener_ui.source(control).unwrap();
                    let old = Intent {control,source,focus_only:true};
                    this.apply_opener(old,window,cx);
                    let key = this.opener_key_callback();
                    this.set_notifications_open(true);this.set_notifications_open(false);
                    let outside=cx.focus_handle();outside.focus(window);
                    this.apply_opener(old,window,cx);
                    assert!(outside.is_focused(window), "hidden-return stale Focus must not steal current focus");
                    this.apply_opener(Intent {focus_only:false,..old},window,cx);
                    assert!(!this.room_details_open && !this.notifications_open && !this.verification_open);
                    let fresh=Intent {source:this.opener_ui.source(control).unwrap(),..old};
                    this.apply_opener(fresh,window,cx);
                    assert!(!key(this,&KeyDownEvent {keystroke:gpui::Keystroke::parse("enter").unwrap(),is_held:false},window,cx));
                    assert!(this.opener_ui.focused(control,window));
                }
                for control in ROOT {
                    for transition in 0..3 {
                        let source=this.opener_ui.source(control).unwrap();
                        let old=Intent {control,source,focus_only:true};
                        let pointer=this.opener_callback(Intent{focus_only:false,..old});
                        let keys=this.opener_key_callback();
                        #[cfg(all(feature="linux-a11y",target_os="linux"))]
                        let semantics=semantic_actions(this,window);
                        let room=this.sidebar.selected().unwrap().scope.clone();
                        match transition {
                            0 => {
                                let other=this.sidebar.rooms().iter().find(|r| r.scope!=room).unwrap().scope.clone();
                                assert!(this.sidebar.select(&other));this.refresh_navigation_visibility();
                                assert!(this.sidebar.select(&room));this.refresh_navigation_visibility();
                            }
                            1 => {
                                let account=this.active_scope.account_id;
                                this.active_scope.account_id=AccountId::new(900);this.refresh_navigation_visibility();
                                this.active_scope.account_id=account;this.refresh_navigation_visibility();
                            }
                            _ => {
                                this.navigation_wide=false;
                                this.set_compact_chat(control!=Control::Info);
                                this.set_compact_chat(control==Control::Info);
                                this.navigation_wide=true;this.set_compact_chat(false);
                            }
                        }
                        let outside=cx.focus_handle();outside.focus(window);
                        this.apply_opener(old,window,cx);
                        assert!(outside.is_focused(window), "room/account/compact return must retire opener source, transition={transition}");
                        pointer(this,window,cx);
                        assert!(!keys(this,&KeyDownEvent{keystroke:gpui::Keystroke::parse("enter").unwrap(),is_held:false},window,cx));
                        #[cfg(all(feature="linux-a11y",target_os="linux"))]
                        for action in &semantics {assert!(this.dispatch_opener_action(action,window,cx));}
                        assert!(outside.is_focused(window));
                        assert!(!this.room_details_open && !this.notifications_open && !this.verification_open);
                    }
                }
                for control in ROOT {
                    let source = this.opener_ui.source(control).unwrap();
                    let click=this.opener_callback(Intent {control,source,focus_only:false});
                    click(this,window,cx);
                    assert_eq!([this.room_details_open,this.verification_open,this.notifications_open],Control::ALL.map(|c| c==control));
                    this.set_room_details_open(false);this.set_verification_open(false);this.set_notifications_open(false);
                    let outside=cx.focus_handle();outside.focus(window);
                    click(this,window,cx);
                    assert!(!this.room_details_open && !this.notifications_open && !this.verification_open);
                    assert!(outside.is_focused(window));
                }
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                {
                    let old=semantic_actions(this,window);
                    for action in old.iter().step_by(2) {
                        assert!(this.dispatch_opener_action(action,window,cx));
                        let matrix_desktop::accessibility::NavigationAction::PanelOpener(intent)=action else {panic!()};
                        assert!(this.opener_ui.focused(intent.control,window));
                    }
                    this.set_notifications_open(true);this.set_notifications_open(false);
                    let outside=cx.focus_handle();outside.focus(window);
                    for action in &old {assert!(this.dispatch_opener_action(action,window,cx));}
                    assert!(outside.is_focused(window));
                    assert!(!this.room_details_open && !this.notifications_open && !this.verification_open);
                    for action in semantic_actions(this,window).iter().step_by(2) {
                        assert!(this.dispatch_opener_action(action,window,cx));
                    }
                }
                eprintln!("native factories plus real SemanticSession/production Shell dispatch passed");
                cx.quit();
            }).unwrap();
        }).detach();
    }
    #[cfg(feature = "live")]
    fn install_availability(window: &mut Window, cx: &mut Context<Shell>) {
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            executor.timer(std::time::Duration::from_millis(300)).await;
            this.update_in(cx, |this, window, cx| {
                use gpui::EntityInputHandler;
                use matrix_engine::signin::{SignInUi, SignInMode, SignInEvent, SignInPrompt};
                this.composer.update(cx, |v, cx| v.replace_text_in_range(Some(0..65536), " bozza 🦀\nP101 ", window, cx));
                this.connected = true;
                this.refresh_navigation_visibility();
                for control in ROOT {
                    let intent = Intent { control, source: this.opener_ui.source(control).unwrap(), focus_only: true };
                    this.apply_opener(intent,window,cx);
                    let focus = this.opener_callback(intent);
                    let click = this.opener_callback(Intent { focus_only: false, ..intent });
                    let key = this.opener_key_callback();
                    #[cfg(all(feature="linux-a11y", target_os="linux"))]
                    let actions = semantic_actions(this,window);
                    this.connected = false;
                    this.refresh_navigation_visibility();
                    assert!(this.opener_ui.sources.iter().all(Option::is_none));
                    #[cfg(all(feature="linux-a11y", target_os="linux"))]
                    assert!(this.semantic_openers(window).is_empty());
                    let outside = cx.focus_handle(); outside.focus(window);
                    for offline in [true,false] {
                        if !offline { this.connected = true; this.refresh_navigation_visibility(); }
                        focus(this,window,cx);click(this,window,cx);
                        assert!(!key(this,&KeyDownEvent { keystroke: gpui::Keystroke::parse("space").unwrap(), is_held: false },window,cx));
                        #[cfg(all(feature="linux-a11y", target_os="linux"))]
                        for action in &actions { assert!(this.dispatch_opener_action(action,window,cx)); }
                        assert!(outside.is_focused(window));
                        assert!(!this.room_details_open && !this.notifications_open && !this.verification_open);
                    }
                }
                let sidebar = std::mem::replace(&mut this.sidebar, Sidebar::connected(this.active_scope.account_id));
                this.refresh_navigation_visibility();
                assert!(this.opener_ui.source(Control::Info).is_none());
                assert!(this.opener_ui.source(Control::Devices).is_none() && this.opener_ui.source(Control::Notifications).is_none());
                this.sidebar = sidebar; this.refresh_navigation_visibility();
                for control in ROOT {
                    for name in ["space", "enter"] {
                        let intent = Intent { control, source: this.opener_ui.source(control).unwrap(), focus_only: true };
                        this.apply_opener(intent,window,cx);
                        let key = this.opener_key_callback();
                        assert!(!key(this,&KeyDownEvent { keystroke: gpui::Keystroke::parse(name).unwrap(), is_held: true },window,cx));
                        assert!(!this.room_details_open && !this.notifications_open && !this.verification_open);
                        assert!(this.opener_ui.focused(control,window));
                        assert!(key(this,&KeyDownEvent { keystroke: gpui::Keystroke::parse(name).unwrap(), is_held: false },window,cx));
                        assert_eq!([this.room_details_open,this.verification_open,this.notifications_open],Control::ALL.map(|c|c==control));
                        assert!(!key(this,&KeyDownEvent { keystroke: gpui::Keystroke::parse(name).unwrap(), is_held: false },window,cx));
                        this.set_room_details_open(false);this.set_verification_open(false);this.set_notifications_open(false);
                    }
                    #[cfg(all(feature="linux-a11y", target_os="linux"))]
                    {
                        let actions = semantic_actions(this,window);
                        let matching = actions.iter().filter(|a| matches!(a,matrix_desktop::accessibility::NavigationAction::PanelOpener(i) if i.control==control)).collect::<Vec<_>>();
                        assert_eq!(matching.len(),2);
                        assert!(this.dispatch_opener_action(matching[0],window,cx));
                        assert!(this.opener_ui.focused(control,window));
                        let next = this.opener_ui.next;
                        assert!(this.dispatch_opener_action(matching[1],window,cx));
                        assert_eq!([this.room_details_open,this.verification_open,this.notifications_open],Control::ALL.map(|c|c==control));
                        assert!(this.dispatch_opener_action(matching[1],window,cx));
                        assert_eq!(this.opener_ui.next,next,"duplicate semantic Click cannot reacquire/open another instance");
                        this.set_room_details_open(false);this.set_verification_open(false);this.set_notifications_open(false);
                        let outside = cx.focus_handle(); outside.focus(window);
                        assert!(this.dispatch_opener_action(matching[1],window,cx));
                        assert!(outside.is_focused(window));
                        assert!(!this.room_details_open && !this.notifications_open && !this.verification_open);
                    }
                }
                eprintln!("offline/empty selection, held keys, Space/Enter and current SemanticSession Focus/Click duplicates characterized");
                for mode in [SignInMode::Password, SignInMode::Unlock] {
                    for control in ROOT {
                        let source = this.opener_ui.source(control).unwrap();
                        let intent = Intent { control, source, focus_only: true };
                        this.apply_opener(intent, window, cx);
                        let focus = this.opener_callback(intent);
                        let click = this.opener_callback(Intent { focus_only: false, ..intent });
                        let key = this.opener_key_callback();
                        #[cfg(all(feature="linux-a11y", target_os="linux"))]
                        let actions = semantic_actions(this, window);
                        let capacity = std::num::NonZeroUsize::new(1).unwrap();
                        let (mut events, event_rx) = matrix_engine::bounded(capacity);
                        let (commands, mut requests) = matrix_engine::bounded(capacity);
                        events.try_send(SignInEvent::Prompt(SignInPrompt { mode, error: None })).unwrap();
                        let view = cx.new(|cx| {
                            let mut view = crate::signin_view::SignInView::new(SignInUi { events: event_rx, setup: matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap()).1, commands }, cx);
                            view.poll(cx);
                            view
                        });
                        this.signin = Some(view);
                        this.refresh_navigation_visibility();
                        assert!(this.opener_ui.source(control).is_none(), "covered sign-in must make root opener unavailable");
                        let outside = cx.focus_handle();
                        outside.focus(window);
                        for covered in [true, false] {
                            if !covered { this.signin = None; this.refresh_navigation_visibility(); }
                            focus(this,window,cx); click(this,window,cx);
                            assert!(!key(this, &KeyDownEvent { keystroke: gpui::Keystroke::parse("enter").unwrap(), is_held: false }, window,cx));
                            #[cfg(all(feature="linux-a11y", target_os="linux"))]
                            for action in &actions { assert!(this.dispatch_opener_action(action,window,cx)); }
                            assert!(outside.is_focused(window));
                            assert!(!this.room_details_open && !this.notifications_open && !this.verification_open);
                        }
                        assert!(requests.try_recv().is_err());
                    }
                }
                this.composer.update(cx, |v,cx| { let mut actual = None; assert_eq!(v.text_for_range(0..65536, &mut actual, window,cx).as_deref(), Some(" bozza 🦀\nP101 ")); });
                eprintln!("Password/Unlock covered and returned roots reject captured native/semantic sources; draft and request queue unchanged");
                cx.quit();
            }).unwrap();
        }).detach();
    }
    #[test]
    #[cfg(feature = "live")]
    #[ignore = "requires private display"]
    fn covered_signin_retires_root_openers_before_dispatch() {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        AVAILABILITY.store(true, Ordering::SeqCst);
        ACTIVE.store(true, Ordering::SeqCst);
        crate::main();
    }
    #[test]
    #[ignore = "requires private display"]
    fn root_openers_focus_traverse_and_activate() {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        ACTIVE.store(true, Ordering::SeqCst);
        crate::main();
    }
}
