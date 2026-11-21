#![allow(clippy::unnecessary_cast)]
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{class, declare_class, msg_send, msg_send_id, mutability, sel, ClassType, DeclaredClass};
use objc2_foundation::{
    CGFloat, CGPoint, CGRect, CGSize, MainThreadMarker, NSArray, NSAttributedStringKey,
    NSComparisonResult, NSDictionary, NSInteger, NSObject, NSRange, NSSet, NSString,
};
use objc2_ui_kit::{
    UIControl, UITextField, UICoordinateSpace, UIEditMenuConfiguration, UIEditMenuInteraction, UIEvent, UIForceTouchCapability,
    UIGestureRecognizer, UIGestureRecognizerDelegate, UIGestureRecognizerState, UIKeyInput,
    UILongPressGestureRecognizer, UIPanGestureRecognizer, UIPasteboard,
    UIPinchGestureRecognizer, UIResponder, UIRotationGestureRecognizer, UITapGestureRecognizer,
    UITextAutocapitalizationType, UITextAutocorrectionType, UITextInput, UITextInputDelegate, UITextInputStringTokenizer,
    UITextInputTokenizer, UITextInputTraits, UITextLayoutDirection, UITextPosition, UITextRange,
    UITextContentTypePassword, UITextContentTypeURL, UITextContentTypeUsername, UITextSelectionRect, UITextSpellCheckingType, UITouch, UITouchPhase, UITouchType,
    UITraitEnvironment, UIView,
};

use super::app_state::{self, EventWrapper};

/// Permanent, cheap trace of the traits handed to UIKit on each focus change (Console / `log stream`).
fn nslog(message: &str) {
    extern "C" {
        fn NSLog(format: &NSString, ...);
    }
    unsafe { NSLog(&NSString::from_str("%@"), &*NSString::from_str(message)) }
}
use super::ime::{downcast_position, downcast_range, WinitTextPosition, WinitTextRange};
use super::ime_document::ImeDocument;
use super::ime_traits::{traits, Credential, Traits};
use super::window::WinitUIWindow;
use crate::dpi::PhysicalPosition;
use crate::event::{ElementState, Event, Force, Ime, KeyEvent, Modifiers, Touch, TouchPhase, WindowEvent};
use crate::keyboard::ModifiersState;
use crate::keyboard::{Key, KeyCode, KeyLocation, NamedKey, NativeKeyCode, PhysicalKey};
use crate::platform_impl::platform::DEVICE_ID;
use crate::platform_impl::KeyEventExtra;
use crate::window::{ImePurpose, WindowAttributes, WindowId as RootWindowId};

pub struct WinitViewState {
    pinch_gesture_recognizer: RefCell<Option<Retained<UIPinchGestureRecognizer>>>,
    doubletap_gesture_recognizer: RefCell<Option<Retained<UITapGestureRecognizer>>>,
    rotation_gesture_recognizer: RefCell<Option<Retained<UIRotationGestureRecognizer>>>,
    pan_gesture_recognizer: RefCell<Option<Retained<UIPanGestureRecognizer>>>,

    // for iOS delta references the start of the Gesture
    rotation_last_delta: Cell<CGFloat>,
    pinch_last_delta: Cell<CGFloat>,
    pan_last_delta: Cell<CGPoint>,

    // `UITextInput`: the text the keyboard sees, driven by its callbacks.
    ime: RefCell<ImeDocument>,
    ime_allowed: Cell<bool>,
    // What the focused field asks of UIKit (content type, no autocorrection, native field for sign-in): see `ime_traits.rs`.
    ime_traits: Cell<Traits>,
    // Real, nearly invisible `UITextField`s (username, password) present while a sign-in field has the focus: UIKit's Passwords
    // AutoFill finds them in the window and fills them with the saved credential, which one custom `UITextInput` view can't take.
    autofill: RefCell<Option<(Retained<WinitAutofillField>, Retained<WinitAutofillField>)>>,
    // The cursor area from `Window::set_ime_cursor_area`, in points; empty until set.
    ime_area: Cell<CGRect>,
    ime_delegate: RefCell<Option<Retained<ProtocolObject<dyn UITextInputDelegate>>>>,
    // One entry per `Ime::Commit` sent: the bytes before the caret it replaces.
    ime_deletes: RefCell<VecDeque<Option<usize>>>,
    // System edit menu (Cut/Copy/Paste/Select All) on long press and double tap in a text field.
    edit_menu: RefCell<Option<Retained<UIEditMenuInteraction>>>,
    edit_menu_at: Cell<CGPoint>,
}

declare_class!(
    /// A real, nearly invisible `UITextField` that UIKit's Passwords AutoFill finds in the window and fills. Its traits are
    /// answered by overrides: UIKit resolves the `UITextInputTraits` setters on `UITextField` at run time.
    pub(crate) struct WinitAutofillField;

    unsafe impl ClassType for WinitAutofillField {
        #[inherits(UIControl, UIView, UIResponder, NSObject)]
        type Super = UITextField;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "WinitAutofillField";
    }

    impl DeclaredClass for WinitAutofillField {
        // The password field (secure, content type password); the other one is the username.
        type Ivars = bool;
    }

    unsafe impl WinitAutofillField {
        #[method_id(textContentType)]
        fn text_content_type(&self) -> Option<Retained<NSString>> {
            unsafe {
                let content = if *self.ivars() { UITextContentTypePassword } else { UITextContentTypeUsername };
                Retained::retain(content as *const NSString as *mut NSString)
            }
        }

        #[method(isSecureTextEntry)]
        fn is_secure_text_entry(&self) -> bool {
            *self.ivars()
        }

        #[method(autocorrectionType)]
        fn autocorrection_type(&self) -> UITextAutocorrectionType {
            UITextAutocorrectionType::No
        }

        #[method(autocapitalizationType)]
        fn autocapitalization_type(&self) -> UITextAutocapitalizationType {
            UITextAutocapitalizationType::None
        }

        // Never takes a touch: egui gets them all through the main view.
        #[method(pointInside:withEvent:)]
        fn point_inside(&self, _point: CGPoint, _event: Option<&UIEvent>) -> bool {
            false
        }

        // What the person types goes to egui through the main view, exactly as for any other field.
        #[method(insertText:)]
        fn insert_text(&self, text: &NSString) {
            let value = text.to_string();
            let _: () = unsafe { msg_send![super(self), insertText: text] };
            if value.chars().count() > 1 {
                // A whole value in one go is Passwords AutoFill (keystrokes arrive one character at a time): it replaces the
                // field, it is not typed into it.
                let value = unsafe { self.text() }.map(|t| t.to_string()).unwrap_or(value);
                nslog(&format!("gnobbi autofill fields: {} {} chars", if *self.ivars() { "password" } else { "username" }, value.chars().count()));
                let mut slot = AUTOFILL.lock().unwrap();
                let (user, pass) = slot.get_or_insert((None, None));
                if *self.ivars() { *pass = Some(value) } else { *user = Some(value) }
                drop(slot);
                unsafe { self.setText(None) };
                if let Some(view) = self.view() {
                    // egui reads the credentials in its next frame.
                    unsafe { let _: () = msg_send![&*view, setNeedsDisplay]; }
                }
            } else if let Some(view) = self.view() {
                view.handle_insert_text(text);
            }
        }

        #[method(deleteBackward)]
        fn delete_backward(&self) {
            if let Some(view) = self.view() {
                view.handle_delete_backward();
            }
            let _: () = unsafe { msg_send![super(self), deleteBackward] };
        }

        // The system edit menu acts on the egui text through the main view.
        #[method(canPerformAction:withSender:)]
        fn can_perform_action(&self, action: Sel, sender: Option<&AnyObject>) -> bool {
            match self.view() {
                Some(view) => unsafe { msg_send![&*view, canPerformAction: action, withSender: sender] },
                None => false,
            }
        }

        #[method(paste:)]
        fn paste(&self, sender: Option<&AnyObject>) {
            if let Some(view) = self.view() {
                let _: () = unsafe { msg_send![&*view, paste: sender] };
            }
        }

        #[method(copy:)]
        fn copy(&self, sender: Option<&AnyObject>) {
            if let Some(view) = self.view() {
                let _: () = unsafe { msg_send![&*view, copy: sender] };
            }
        }

        #[method(cut:)]
        fn cut(&self, sender: Option<&AnyObject>) {
            if let Some(view) = self.view() {
                let _: () = unsafe { msg_send![&*view, cut: sender] };
            }
        }

        #[method(selectAll:)]
        fn select_all(&self, sender: Option<&AnyObject>) {
            if let Some(view) = self.view() {
                let _: () = unsafe { msg_send![&*view, selectAll: sender] };
            }
        }
    }
);

impl WinitAutofillField {
    fn view(&self) -> Option<Retained<WinitView>> {
        // Only `WinitView::sync_autofill_fields` adds these fields, always to a `WinitView`.
        unsafe { self.superview().map(|view| Retained::cast::<WinitView>(view)) }
    }
}

declare_class!(
    pub(crate) struct WinitView;

    unsafe impl ClassType for WinitView {
        #[inherits(UIResponder, NSObject)]
        type Super = UIView;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "WinitUIView";
    }

    impl DeclaredClass for WinitView {
        type Ivars = WinitViewState;
    }

    unsafe impl WinitView {
        #[method(drawRect:)]
        fn draw_rect(&self, rect: CGRect) {
            let mtm = MainThreadMarker::new().unwrap();
            let window = self.window().unwrap();
            app_state::handle_nonuser_event(
                mtm,
                EventWrapper::StaticEvent(Event::WindowEvent {
                    window_id: RootWindowId(window.id()),
                    event: WindowEvent::RedrawRequested,
                }),
            );
            let _: () = unsafe { msg_send![super(self), drawRect: rect] };
        }

        #[method(layoutSubviews)]
        fn layout_subviews(&self) {
            let mtm = MainThreadMarker::new().unwrap();
            let _: () = unsafe { msg_send![super(self), layoutSubviews] };

            let window = self.window().unwrap();
            let window_bounds = window.bounds();
            let screen = window.screen();
            let screen_space = screen.coordinateSpace();
            let screen_frame = self.convertRect_toCoordinateSpace(window_bounds, &screen_space);
            let scale_factor = screen.scale();
            let size = crate::dpi::LogicalSize {
                width: screen_frame.size.width as f64,
                height: screen_frame.size.height as f64,
            }
            .to_physical(scale_factor as f64);

            // If the app is started in landscape, the view frame and window bounds can be mismatched.
            // The view frame will be in portrait and the window bounds in landscape. So apply the
            // window bounds to the view frame to make it consistent.
            let view_frame = self.frame();
            if view_frame != window_bounds {
                self.setFrame(window_bounds);
            }

            app_state::handle_nonuser_event(
                mtm,
                EventWrapper::StaticEvent(Event::WindowEvent {
                    window_id: RootWindowId(window.id()),
                    event: WindowEvent::Resized(size),
                }),
            );
        }

        #[method(setContentScaleFactor:)]
        fn set_content_scale_factor(&self, untrusted_scale_factor: CGFloat) {
            let mtm = MainThreadMarker::new().unwrap();
            let _: () =
                unsafe { msg_send![super(self), setContentScaleFactor: untrusted_scale_factor] };

            // `window` is null when `setContentScaleFactor` is invoked prior to `[UIWindow
            // makeKeyAndVisible]` at window creation time (either manually or internally by
            // UIKit when the `UIView` is first created), in which case we send no events here
            let window = match self.window() {
                Some(window) => window,
                None => return,
            };
            // `setContentScaleFactor` may be called with a value of 0, which means "reset the
            // content scale factor to a device-specific default value", so we can't use the
            // parameter here. We can query the actual factor using the getter
            let scale_factor = self.contentScaleFactor();
            assert!(
                !scale_factor.is_nan()
                    && scale_factor.is_finite()
                    && scale_factor.is_sign_positive()
                    && scale_factor > 0.0,
                "invalid scale_factor set on UIView",
            );
            let scale_factor = scale_factor as f64;
            let bounds = self.bounds();
            let screen = window.screen();
            let screen_space = screen.coordinateSpace();
            let screen_frame = self.convertRect_toCoordinateSpace(bounds, &screen_space);
            let size = crate::dpi::LogicalSize {
                width: screen_frame.size.width as f64,
                height: screen_frame.size.height as f64,
            };
            let window_id = RootWindowId(window.id());
            app_state::handle_nonuser_events(
                mtm,
                std::iter::once(EventWrapper::ScaleFactorChanged(
                    app_state::ScaleFactorChanged {
                        window,
                        scale_factor,
                        suggested_size: size.to_physical(scale_factor),
                    },
                ))
                .chain(std::iter::once(EventWrapper::StaticEvent(
                    Event::WindowEvent {
                        window_id,
                        event: WindowEvent::Resized(size.to_physical(scale_factor)),
                    },
                ))),
            );
        }

        #[method(touchesBegan:withEvent:)]
        fn touches_began(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            // A tap ends the composition, as in `UITextField`, and may move the caret.
            self.commit_marked();
            self.reset_ime();
            self.handle_touches(touches)
        }

        #[method(touchesMoved:withEvent:)]
        fn touches_moved(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            self.handle_touches(touches)
        }

        #[method(touchesEnded:withEvent:)]
        fn touches_ended(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            self.handle_touches(touches)
        }

        #[method(touchesCancelled:withEvent:)]
        fn touches_cancelled(&self, touches: &NSSet<UITouch>, _event: Option<&UIEvent>) {
            self.handle_touches(touches)
        }

        #[method(pinchGesture:)]
        fn pinch_gesture(&self, recognizer: &UIPinchGestureRecognizer) {
            let window = self.window().unwrap();

            let (phase, delta) = match recognizer.state() {
                UIGestureRecognizerState::Began => {
                    self.ivars().pinch_last_delta.set(recognizer.scale());
                    (TouchPhase::Started, 0.0)
                }
                UIGestureRecognizerState::Changed => {
                    let last_scale: f64 = self.ivars().pinch_last_delta.replace(recognizer.scale());
                    (TouchPhase::Moved, recognizer.scale() - last_scale)
                }
                UIGestureRecognizerState::Ended => {
                    let last_scale: f64 = self.ivars().pinch_last_delta.replace(0.0);
                    (TouchPhase::Moved, recognizer.scale() - last_scale)
                }
                UIGestureRecognizerState::Cancelled | UIGestureRecognizerState::Failed => {
                    self.ivars().rotation_last_delta.set(0.0);
                    // Pass -delta so that action is reversed
                    (TouchPhase::Cancelled, -recognizer.scale())
                }
                state => panic!("unexpected recognizer state: {state:?}"),
            };

            let gesture_event = EventWrapper::StaticEvent(Event::WindowEvent {
                window_id: RootWindowId(window.id()),
                event: WindowEvent::PinchGesture {
                    device_id: DEVICE_ID,
                    delta: delta as f64,
                    phase,
                },
            });

            let mtm = MainThreadMarker::new().unwrap();
            app_state::handle_nonuser_event(mtm, gesture_event);
        }

        #[method(doubleTapGesture:)]
        fn double_tap_gesture(&self, recognizer: &UITapGestureRecognizer) {
            let window = self.window().unwrap();

            if recognizer.state() == UIGestureRecognizerState::Ended {
                let gesture_event = EventWrapper::StaticEvent(Event::WindowEvent {
                    window_id: RootWindowId(window.id()),
                    event: WindowEvent::DoubleTapGesture {
                        device_id: DEVICE_ID,
                    },
                });

                let mtm = MainThreadMarker::new().unwrap();
                app_state::handle_nonuser_event(mtm, gesture_event);
            }
        }

        #[method(rotationGesture:)]
        fn rotation_gesture(&self, recognizer: &UIRotationGestureRecognizer) {
            let window = self.window().unwrap();

            let (phase, delta) = match recognizer.state() {
                UIGestureRecognizerState::Began => {
                    self.ivars().rotation_last_delta.set(0.0);

                    (TouchPhase::Started, 0.0)
                }
                UIGestureRecognizerState::Changed => {
                    let last_rotation = self.ivars().rotation_last_delta.replace(recognizer.rotation());

                    (TouchPhase::Moved, recognizer.rotation() - last_rotation)
                }
                UIGestureRecognizerState::Ended => {
                    let last_rotation = self.ivars().rotation_last_delta.replace(0.0);

                    (TouchPhase::Ended, recognizer.rotation() - last_rotation)
                }
                UIGestureRecognizerState::Cancelled | UIGestureRecognizerState::Failed => {
                    self.ivars().rotation_last_delta.set(0.0);

                    // Pass -delta so that action is reversed
                    (TouchPhase::Cancelled, -recognizer.rotation())
                }
                state => panic!("unexpected recognizer state: {state:?}"),
            };

            // Make delta negative to match macos, convert to degrees
            let gesture_event = EventWrapper::StaticEvent(Event::WindowEvent {
                window_id: RootWindowId(window.id()),
                event: WindowEvent::RotationGesture {
                    device_id: DEVICE_ID,
                    delta: -delta.to_degrees() as _,
                    phase,
                },
            });

            let mtm = MainThreadMarker::new().unwrap();
            app_state::handle_nonuser_event(mtm, gesture_event);
        }

        #[method(panGesture:)]
        fn pan_gesture(&self, recognizer: &UIPanGestureRecognizer) {
            let window = self.window().unwrap();

            let translation = recognizer.translationInView(Some(self));

            let (phase, dx, dy) = match recognizer.state() {
                UIGestureRecognizerState::Began => {
                    self.ivars().pan_last_delta.set(translation);

                    (TouchPhase::Started, 0.0, 0.0)
                }
                UIGestureRecognizerState::Changed => {
                    let last_pan: CGPoint = self.ivars().pan_last_delta.replace(translation);

                    let dx = translation.x - last_pan.x;
                    let dy = translation.y - last_pan.y;

                    (TouchPhase::Moved, dx, dy)
                }
                UIGestureRecognizerState::Ended => {
                    let last_pan: CGPoint = self.ivars().pan_last_delta.replace(CGPoint{x:0.0, y:0.0});

                    let dx = translation.x - last_pan.x;
                    let dy = translation.y - last_pan.y;

                    (TouchPhase::Ended, dx, dy)
                }
                UIGestureRecognizerState::Cancelled | UIGestureRecognizerState::Failed => {
                    let last_pan: CGPoint = self.ivars().pan_last_delta.replace(CGPoint{x:0.0, y:0.0});

                    // Pass -delta so that action is reversed
                    (TouchPhase::Cancelled, -last_pan.x, -last_pan.y)
                }
                state => panic!("unexpected recognizer state: {state:?}"),
            };


            let gesture_event = EventWrapper::StaticEvent(Event::WindowEvent {
                window_id: RootWindowId(window.id()),
                event: WindowEvent::PanGesture {
                    device_id: DEVICE_ID,
                    delta: PhysicalPosition::new(dx as _, dy as _),
                    phase,
                },
            });

            let mtm = MainThreadMarker::new().unwrap();
            app_state::handle_nonuser_event(mtm, gesture_event);
        }

        // System edit actions: the edit menu and the hardware Cmd+C/X/V/A reach the view through the responder chain.
        #[method(canPerformAction:withSender:)]
        fn can_perform_action(&self, action: Sel, sender: Option<&AnyObject>) -> bool {
            if action == sel!(paste:) {
                self.ivars().ime_allowed.get() && unsafe { UIPasteboard::generalPasteboard().hasStrings() }
            } else if action == sel!(selectAll:) {
                self.ivars().ime_allowed.get()
            } else if action == sel!(copy:) || action == sel!(cut:) {
                // Nothing leaves a password field.
                self.ivars().ime_allowed.get() && !self.ivars().ime_traits.get().secret
            } else {
                unsafe { msg_send![super(self), canPerformAction: action, withSender: sender] }
            }
        }

        #[method(paste:)]
        fn paste(&self, _sender: Option<&AnyObject>) {
            // Same path as typing, so it reaches whichever field has the focus (password included).
            if let Some(text) = unsafe { UIPasteboard::generalPasteboard().string() } {
                self.handle_insert_text(&text);
            }
        }

        #[method(copy:)]
        fn copy(&self, _sender: Option<&AnyObject>) {
            self.emit_named_key(NamedKey::Copy);
        }

        #[method(cut:)]
        fn cut(&self, _sender: Option<&AnyObject>) {
            self.emit_named_key(NamedKey::Cut);
        }

        #[method(selectAll:)]
        fn select_all(&self, _sender: Option<&AnyObject>) {
            // Cmd+A, as a hardware keyboard would send it.
            self.emit_window_event(WindowEvent::ModifiersChanged(Modifiers::from(ModifiersState::SUPER)));
            self.emit_key(Key::Character("a".into()), PhysicalKey::Code(KeyCode::KeyA));
            self.emit_window_event(WindowEvent::ModifiersChanged(Modifiers::default()));
            // UIKit closes the menu after an action; iOS shows it again on the selection (Cut/Copy/Paste), so do the same.
            unsafe { let _: () = msg_send![self, performSelector: sel!(reshowEditMenu), withObject: None::<&AnyObject>, afterDelay: 0.2f64]; }
        }

        #[method(reshowEditMenu)]
        fn reshow_edit_menu(&self) {
            self.present_edit_menu(self.ivars().edit_menu_at.get());
        }

        #[method(showEditMenu:)]
        fn show_edit_menu(&self, recognizer: &UIGestureRecognizer) {
            // Long press starts the menu; the double tap ends after egui has selected the word.
            let state = recognizer.state();
            if state != UIGestureRecognizerState::Began && state != UIGestureRecognizerState::Ended {
                return;
            }
            if !self.ivars().ime_allowed.get() {
                return;
            }
            let at = unsafe { recognizer.locationInView(Some(self)) };
            // shortcut: the field is known only by the caret rectangle, so a press within 60 pt above or below it counts as on
            // the field; a field that wraps over more lines than that does not get the menu on its first lines.
            let caret = self.ivars().ime_area.get();
            if caret.size.height > 0.0 && (at.y < caret.origin.y - 60.0 || at.y > caret.origin.y + caret.size.height + 60.0) {
                return;
            }
            self.present_edit_menu(at);
        }

        #[method(canBecomeFirstResponder)]
        fn can_become_first_responder(&self) -> bool {
            true
        }
    }

    unsafe impl NSObjectProtocol for WinitView {}

    unsafe impl UIGestureRecognizerDelegate for WinitView {
        #[method(gestureRecognizer:shouldRecognizeSimultaneouslyWithGestureRecognizer:)]
        fn should_recognize_simultaneously(&self, _gesture_recognizer: &UIGestureRecognizer, _other_gesture_recognizer: &UIGestureRecognizer) -> bool {
            true
        }
    }

    unsafe impl UITextInputTraits for WinitView {
        // Always false on the main view: once UIKit has seen it true here it keeps the Passwords bar on every later field. The
        // sign-in password is a real secure `UITextField` (`WinitAutofillField`) while it has the focus.
        #[method(isSecureTextEntry)]
        fn is_secure_text_entry(&self) -> bool {
            false
        }

        #[method_id(textContentType)]
        fn text_content_type(&self) -> Option<Retained<NSString>> {
            // Unspecified (nil) except for the server address.
            if self.ivars().ime_traits.get().url {
                unsafe { Retained::retain(UITextContentTypeURL as *const NSString as *mut NSString) }
            } else {
                None
            }
        }

        #[method(autocorrectionType)]
        fn autocorrection_type(&self) -> UITextAutocorrectionType {
            if self.ivars().ime_traits.get().raw {
                UITextAutocorrectionType::No
            } else {
                UITextAutocorrectionType::Default
            }
        }

        #[method(autocapitalizationType)]
        fn autocapitalization_type(&self) -> UITextAutocapitalizationType {
            if self.ivars().ime_traits.get().raw {
                UITextAutocapitalizationType::None
            } else {
                UITextAutocapitalizationType::Sentences
            }
        }

        #[method(spellCheckingType)]
        fn spell_checking_type(&self) -> UITextSpellCheckingType {
            if self.ivars().ime_traits.get().raw {
                UITextSpellCheckingType::No
            } else {
                UITextSpellCheckingType::Default
            }
        }
    }

    unsafe impl UIKeyInput for WinitView {
        #[method(hasText)]
        fn has_text(&self) -> bool {
            true
        }

        #[method(insertText:)]
        fn insert_text(&self, text: &NSString) {
            self.handle_insert_text(text)
        }

        #[method(deleteBackward)]
        fn delete_backward(&self) {
            self.handle_delete_backward()
        }
    }

    // UIKit needs `UITextInput` on top of `UIKeyInput` to run multi-stage input methods (Chinese,
    // Japanese, Korean, ...) and to autocorrect: without it the keyboard drops composed text.
    // Adapted from rust-windowing/winit PR #4592 (head e49d771a). The "document" it reports is
    // `ImeDocument`: the text typed since the caret last moved, then the marked text.
    unsafe impl UITextInput for WinitView {
        // --- marked text (preedit) -----------------------------------

        #[method(setMarkedText:selectedRange:)]
        unsafe fn set_marked_text(&self, marked_text: Option<&NSString>, selected_range: NSRange) {
            let text = marked_text.map(|s| s.to_string()).unwrap_or_default();
            // nil or empty removes the composition (the last letter was deleted): it isn't committed.
            if text.is_empty() {
                self.discard_marked();
                return;
            }
            let was_marked = !self.ivars().ime.borrow().marked().is_empty();
            self.ivars()
                .ime
                .borrow_mut()
                .set_marked(text, (selected_range.location, selected_range.length));
            if !was_marked {
                self.emit_ime_event(Ime::Enabled);
            }
            self.emit_preedit();
        }

        #[method(unmarkText)]
        unsafe fn unmark_text(&self) {
            // The marked text becomes real text, as in `UITextField`.
            self.commit_marked();
        }

        #[method_id(markedTextRange)]
        unsafe fn marked_text_range(&self) -> Option<Retained<UITextRange>> {
            let doc = self.ivars().ime.borrow();
            let mtm = MainThreadMarker::new().unwrap();
            (!doc.marked().is_empty()).then(|| {
                Retained::into_super(WinitTextRange::new(
                    mtm,
                    doc.caret() as i64,
                    doc.end() as i64,
                ))
            })
        }

        #[method_id(markedTextStyle)]
        unsafe fn marked_text_style(
            &self,
        ) -> Option<Retained<NSDictionary<NSAttributedStringKey, AnyObject>>> {
            None
        }

        #[method(setMarkedTextStyle:)]
        unsafe fn set_marked_text_style(
            &self,
            _: Option<&NSDictionary<NSAttributedStringKey, AnyObject>>,
        ) {
        }

        // --- text storage queries --------------------------------------

        #[method_id(textInRange:)]
        unsafe fn text_in_range(&self, range: &UITextRange) -> Option<Retained<NSString>> {
            downcast_range(range).map(|range| {
                let (start, end) = (range.start_offset().max(0), range.end_offset().max(0));
                let text = if self.ivars().ime_traits.get().secret {
                    None
                } else {
                    self.ivars()
                        .ime
                        .borrow()
                        .substring((start as usize, end.saturating_sub(start) as usize))
                };
                NSString::from_str(text.as_ref().map_or("", |(text, _)| text))
            })
        }

        #[method(replaceRange:withText:)]
        unsafe fn replace_range_with_text(&self, range: &UITextRange, text: &NSString) {
            // Autocorrection and candidate selection: the range reaches the caret (see
            // `ImeDocument::replace`). The text replaces it as a commit that deletes the replaced
            // text, once.
            let Some(range) = downcast_range(range) else { return };
            let (start, end) = (range.start_offset().max(0) as usize, range.end_offset().max(0) as usize);
            let was_marked = !self.ivars().ime.borrow().marked().is_empty();
            let Some(delete) = self.ivars().ime.borrow_mut().replace((start, end.saturating_sub(start))) else {
                return;
            };
            if was_marked {
                self.emit_ime_event(Ime::Preedit(String::new(), None));
            }
            let text = text.to_string();
            if !text.is_empty() || delete > 0 {
                self.type_text(&text);
                self.queue_commit(text, Some(delete).filter(|&n| n > 0));
            }
        }

        // --- selection ---------------------------------------------------

        #[method_id(selectedTextRange)]
        unsafe fn selected_text_range(&self) -> Option<Retained<UITextRange>> {
            let (location, length) = self.ivars().ime.borrow().selected_range();
            let mtm = MainThreadMarker::new().unwrap();
            Some(Retained::into_super(WinitTextRange::new(
                mtm,
                location as i64,
                (location + length) as i64,
            )))
        }

        #[method(setSelectedTextRange:)]
        unsafe fn set_selected_text_range(&self, range: Option<&UITextRange>) {
            let Some(range) = range.and_then(|r| downcast_range(r)) else { return };
            let (start, end) = (range.start_offset().max(0) as usize, range.end_offset().max(0) as usize);
            self.ivars().ime.borrow_mut().select((start, end.saturating_sub(start)));
        }

        // --- document boundaries -----------------------------------------

        #[method_id(beginningOfDocument)]
        unsafe fn beginning_of_document(&self) -> Retained<UITextPosition> {
            let mtm = MainThreadMarker::new().unwrap();
            let start = self.ivars().ime.borrow().start() as i64;
            Retained::into_super(WinitTextPosition::new(mtm, start))
        }

        #[method_id(endOfDocument)]
        unsafe fn end_of_document(&self) -> Retained<UITextPosition> {
            let mtm = MainThreadMarker::new().unwrap();
            let end = self.ivars().ime.borrow().end() as i64;
            Retained::into_super(WinitTextPosition::new(mtm, end))
        }

        // --- position / range arithmetic -----------------------------------

        #[method_id(textRangeFromPosition:toPosition:)]
        unsafe fn text_range_from_position(
            &self,
            from: &UITextPosition,
            to: &UITextPosition,
        ) -> Option<Retained<UITextRange>> {
            let mtm = MainThreadMarker::new().unwrap();
            let offsets = downcast_position(from).zip(downcast_position(to));
            offsets.map(|(f, t)| {
                let (f, t) = (f.offset(), t.offset());
                Retained::into_super(WinitTextRange::new(mtm, f.min(t), f.max(t)))
            })
        }

        #[method_id(positionFromPosition:offset:)]
        unsafe fn position_from_position_offset(
            &self,
            position: &UITextPosition,
            offset: NSInteger,
        ) -> Option<Retained<UITextPosition>> {
            self.moved_position(position, offset as i64)
        }

        #[method_id(positionFromPosition:inDirection:offset:)]
        unsafe fn position_from_position_in_direction_offset(
            &self,
            position: &UITextPosition,
            direction: UITextLayoutDirection,
            offset: NSInteger,
        ) -> Option<Retained<UITextPosition>> {
            // Right (2) and down (3) move forward; left (1) and up (0) backward.
            let signed = match direction.0 {
                2 | 3 => offset as i64,
                _ => -(offset as i64),
            };
            self.moved_position(position, signed)
        }

        #[method(comparePosition:toPosition:)]
        unsafe fn compare_position_to_position(
            &self,
            position: &UITextPosition,
            other: &UITextPosition,
        ) -> NSComparisonResult {
            let a = downcast_position(position).map(|p| p.offset()).unwrap_or(0);
            let b = downcast_position(other).map(|p| p.offset()).unwrap_or(0);
            if a < b {
                NSComparisonResult::Ascending
            } else if a > b {
                NSComparisonResult::Descending
            } else {
                NSComparisonResult::Same
            }
        }

        #[method(offsetFromPosition:toPosition:)]
        unsafe fn offset_from_position_to_position(
            &self,
            from: &UITextPosition,
            to: &UITextPosition,
        ) -> NSInteger {
            let a = downcast_position(from).map(|p| p.offset()).unwrap_or(0);
            let b = downcast_position(to).map(|p| p.offset()).unwrap_or(0);
            (b - a) as NSInteger
        }

        // --- delegate / tokenizer ---------------------------------------

        #[method_id(inputDelegate)]
        unsafe fn input_delegate(
            &self,
        ) -> Option<Retained<ProtocolObject<dyn UITextInputDelegate>>> {
            self.ivars().ime_delegate.borrow().as_deref().map(retained)
        }

        #[method(setInputDelegate:)]
        unsafe fn set_input_delegate(
            &self,
            delegate: Option<&ProtocolObject<dyn UITextInputDelegate>>,
        ) {
            *self.ivars().ime_delegate.borrow_mut() = delegate.map(retained);
        }

        #[method_id(tokenizer)]
        unsafe fn tokenizer(&self) -> Retained<ProtocolObject<dyn UITextInputTokenizer>> {
            let mtm = MainThreadMarker::new().unwrap();
            let responder: &UIResponder = self.as_super();
            let tokenizer =
                unsafe { UITextInputStringTokenizer::initWithTextInput(mtm.alloc(), responder) };
            ProtocolObject::from_retained(tokenizer)
        }

        // --- direction queries ----------------------------------------

        #[method_id(positionWithinRange:farthestInDirection:)]
        unsafe fn position_within_range_in_direction(
            &self,
            range: &UITextRange,
            direction: UITextLayoutDirection,
        ) -> Option<Retained<UITextPosition>> {
            let mtm = MainThreadMarker::new().unwrap();
            downcast_range(range).map(|r| {
                let off = match direction.0 {
                    2 | 3 => r.end_offset(),
                    _ => r.start_offset(),
                };
                Retained::into_super(WinitTextPosition::new(mtm, off))
            })
        }

        #[method_id(characterRangeByExtendingPosition:inDirection:)]
        unsafe fn character_range_by_extending(
            &self,
            position: &UITextPosition,
            direction: UITextLayoutDirection,
        ) -> Option<Retained<UITextRange>> {
            let (start, end) = {
                let doc = self.ivars().ime.borrow();
                (doc.start() as i64, doc.end() as i64)
            };
            let mtm = MainThreadMarker::new().unwrap();
            downcast_position(position).map(|p| {
                let pos = p.offset();
                let (from, to) = match direction.0 {
                    2 | 3 => (pos, end),
                    _ => (start, pos),
                };
                Retained::into_super(WinitTextRange::new(mtm, from, to))
            })
        }

        // --- geometry: the candidate bar sits at the area from `set_ime_cursor_area` ----

        #[method(firstRectForRange:)]
        unsafe fn first_rect_for_range(&self, _range: &UITextRange) -> CGRect {
            self.ime_rect()
        }

        #[method(caretRectForPosition:)]
        unsafe fn caret_rect_for_position(&self, _position: &UITextPosition) -> CGRect {
            self.ime_rect()
        }

        #[method_id(selectionRectsForRange:)]
        unsafe fn selection_rects_for_range(
            &self,
            _range: &UITextRange,
        ) -> Retained<NSArray<UITextSelectionRect>> {
            NSArray::new()
        }

        // --- hit testing ---------------------------------------------

        #[method_id(closestPositionToPoint:)]
        unsafe fn closest_position_to_point(
            &self,
            _point: CGPoint,
        ) -> Option<Retained<UITextPosition>> {
            let mtm = MainThreadMarker::new().unwrap();
            let caret = self.ivars().ime.borrow().caret() as i64;
            Some(Retained::into_super(WinitTextPosition::new(mtm, caret)))
        }

        #[method_id(closestPositionToPoint:withinRange:)]
        unsafe fn closest_position_to_point_within(
            &self,
            _point: CGPoint,
            range: &UITextRange,
        ) -> Option<Retained<UITextPosition>> {
            let mtm = MainThreadMarker::new().unwrap();
            let off = downcast_range(range).map(|r| r.start_offset()).unwrap_or(0);
            Some(Retained::into_super(WinitTextPosition::new(mtm, off)))
        }

        #[method_id(characterRangeAtPoint:)]
        unsafe fn character_range_at_point(
            &self,
            _point: CGPoint,
        ) -> Option<Retained<UITextRange>> {
            let mtm = MainThreadMarker::new().unwrap();
            let caret = self.ivars().ime.borrow().caret() as i64;
            Some(Retained::into_super(WinitTextRange::new(mtm, caret, caret)))
        }

        // Required by the protocol, but the `NSWritingDirection` binding sits behind the `NSText`
        // feature: declare them against `NSInteger` (natural direction == 0).
        #[method(baseWritingDirectionForPosition:inDirection:)]
        unsafe fn base_writing_direction(
            &self,
            _position: &UITextPosition,
            _direction: NSInteger,
        ) -> NSInteger {
            0
        }

        #[method(setBaseWritingDirection:forRange:)]
        unsafe fn set_base_writing_direction(&self, _direction: NSInteger, _range: &UITextRange) {}
    }
);

/// What AutoFill put into the sign-in fields: (username, password), `None` for a field it left empty.
static AUTOFILL: std::sync::Mutex<Option<(Option<String>, Option<String>)>> = std::sync::Mutex::new(None);

pub(crate) fn take_autofill_credentials() -> Option<(Option<String>, Option<String>)> {
    AUTOFILL.lock().unwrap().take()
}

impl WinitView {
    /// The sign-in fields are real, hidden `UITextField`s (username, password) while a sign-in field has the focus: UIKit's
    /// Passwords AutoFill fills a native pair together and offers its suggestions per first responder. They are attached
    /// when a sign-in field takes the focus and dropped, with their text, when another kind of field does.
    fn sync_autofill_fields(&self, want: bool) {
        let mut fields = self.ivars().autofill.borrow_mut();
        if want && fields.is_none() {
            let mtm = MainThreadMarker::new().unwrap();
            let make = |secure: bool| -> Retained<WinitAutofillField> {
                unsafe {
                    let this = mtm.alloc().set_ivars(secure);
                    let field: Retained<WinitAutofillField> = msg_send_id![super(this), initWithFrame: CGRect {
                        origin: CGPoint { x: 0.0, y: 0.0 },
                        size: CGSize { width: 120.0, height: 30.0 },
                    }];
                    field.setAlpha(0.02);
                    self.addSubview(&field);
                    field
                }
            };
            *fields = Some((make(false), make(true)));
        } else if !want {
            if let Some((user, pass)) = fields.take() {
                for field in [user, pass] {
                    unsafe { field.removeFromSuperview(); }
                }
            }
        }
    }

    /// The native field that stands in for the focused sign-in field, if one does.
    fn credential_field(&self) -> Option<Retained<WinitAutofillField>> {
        let credential = self.ivars().ime_traits.get().credential?;
        self.ivars().autofill.borrow().as_ref().map(|(user, pass)| if credential == Credential::Username { user } else { pass }.clone())
    }

    /// Show the keyboard for whichever view takes the input: the native field of a sign-in field, the view itself otherwise.
    pub(crate) fn become_input_responder(&self) {
        unsafe {
            match self.credential_field() {
                Some(field) => field.becomeFirstResponder(),
                None => self.becomeFirstResponder(),
            };
        }
    }

    pub(crate) fn resign_input_responder(&self) {
        unsafe {
            if let Some((user, pass)) = self.ivars().autofill.borrow().as_ref() {
                user.resignFirstResponder();
                pass.resignFirstResponder();
            }
            self.resignFirstResponder();
        }
    }

    pub(crate) fn new(
        mtm: MainThreadMarker,
        window_attributes: &WindowAttributes,
        frame: CGRect,
    ) -> Retained<Self> {
        let this = mtm.alloc().set_ivars(WinitViewState {
            pinch_gesture_recognizer: RefCell::new(None),
            doubletap_gesture_recognizer: RefCell::new(None),
            rotation_gesture_recognizer: RefCell::new(None),
            pan_gesture_recognizer: RefCell::new(None),

            rotation_last_delta: Cell::new(0.0),
            pinch_last_delta: Cell::new(0.0),
            pan_last_delta: Cell::new(CGPoint { x: 0.0, y: 0.0 }),

            ime: RefCell::new(ImeDocument::default()),
            ime_allowed: Cell::new(false),
            ime_traits: Cell::new(traits(ImePurpose::Normal)),
            autofill: RefCell::new(None),
            ime_area: Cell::new(CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: 0.0, height: 0.0 } }),
            ime_delegate: RefCell::new(None),
            ime_deletes: RefCell::new(VecDeque::new()),
            edit_menu: RefCell::new(None),
            edit_menu_at: Cell::new(CGPoint { x: 0.0, y: 0.0 }),
        });
        let this: Retained<Self> = unsafe { msg_send_id![super(this), initWithFrame: frame] };

        this.setMultipleTouchEnabled(true);
        this.install_edit_menu(mtm);

        if let Some(scale_factor) = window_attributes.platform_specific.scale_factor {
            this.setContentScaleFactor(scale_factor as _);
        }

        this
    }

    /// Long press and double tap open the system edit menu (the interaction is shown by `showEditMenu:`).
    fn install_edit_menu(&self, mtm: MainThreadMarker) {
        unsafe {
            let menu = UIEditMenuInteraction::initWithDelegate(mtm.alloc(), None);
            self.addInteraction(ProtocolObject::from_ref(&*menu));
            self.ivars().edit_menu.replace(Some(menu));
            let long = UILongPressGestureRecognizer::initWithTarget_action(mtm.alloc(), Some(self), Some(sel!(showEditMenu:)));
            let tap = UITapGestureRecognizer::initWithTarget_action(mtm.alloc(), Some(self), Some(sel!(showEditMenu:)));
            tap.setNumberOfTapsRequired(2);
            for r in [&**long as &UIGestureRecognizer, &**tap as &UIGestureRecognizer] {
                // egui keeps every touch: the menu is an extra, not a replacement.
                r.setCancelsTouchesInView(false);
                r.setDelaysTouchesEnded(false);
                r.setDelegate(Some(ProtocolObject::from_ref(self)));
                self.addGestureRecognizer(r);
            }
        }
    }

    fn present_edit_menu(&self, at: CGPoint) {
        self.ivars().edit_menu_at.set(at);
        self.become_input_responder();
        if let Some(menu) = self.ivars().edit_menu.borrow().as_ref() {
            let mtm = MainThreadMarker::from(self);
            unsafe {
                let config = UIEditMenuConfiguration::configurationWithIdentifier_sourcePoint(None, at, mtm);
                menu.presentEditMenuWithConfiguration(&config);
            }
        }
    }

    fn emit_key(&self, logical_key: Key, physical_key: PhysicalKey) {
        for state in [ElementState::Pressed, ElementState::Released] {
            self.emit_window_event(WindowEvent::KeyboardInput {
                device_id: DEVICE_ID,
                event: KeyEvent {
                    state,
                    logical_key: logical_key.clone(),
                    physical_key,
                    platform_specific: KeyEventExtra {},
                    repeat: false,
                    location: KeyLocation::Standard,
                    text: None,
                },
                is_synthetic: false,
            });
        }
    }

    fn emit_named_key(&self, key: NamedKey) {
        self.emit_key(Key::Named(key), PhysicalKey::Unidentified(NativeKeyCode::Unidentified));
    }

    fn window(&self) -> Option<Retained<WinitUIWindow>> {
        // SAFETY: `WinitView`s are always installed in a `WinitUIWindow`
        (**self).window().map(|window| unsafe { Retained::cast(window) })
    }

    pub(crate) fn recognize_pinch_gesture(&self, should_recognize: bool) {
        let mtm = MainThreadMarker::from(self);
        if should_recognize {
            if self.ivars().pinch_gesture_recognizer.borrow().is_none() {
                let pinch = unsafe {
                    UIPinchGestureRecognizer::initWithTarget_action(
                        mtm.alloc(),
                        Some(self),
                        Some(sel!(pinchGesture:)),
                    )
                };
                pinch.setDelegate(Some(ProtocolObject::from_ref(self)));
                self.addGestureRecognizer(&pinch);
                self.ivars().pinch_gesture_recognizer.replace(Some(pinch));
            }
        } else if let Some(recognizer) = self.ivars().pinch_gesture_recognizer.take() {
            self.removeGestureRecognizer(&recognizer);
        }
    }

    pub(crate) fn recognize_pan_gesture(
        &self,
        should_recognize: bool,
        minimum_number_of_touches: u8,
        maximum_number_of_touches: u8,
    ) {
        let mtm = MainThreadMarker::from(self);
        if should_recognize {
            if self.ivars().pan_gesture_recognizer.borrow().is_none() {
                let pan = unsafe {
                    UIPanGestureRecognizer::initWithTarget_action(
                        mtm.alloc(),
                        Some(self),
                        Some(sel!(panGesture:)),
                    )
                };
                pan.setDelegate(Some(ProtocolObject::from_ref(self)));
                pan.setMinimumNumberOfTouches(minimum_number_of_touches as _);
                pan.setMaximumNumberOfTouches(maximum_number_of_touches as _);
                self.addGestureRecognizer(&pan);
                self.ivars().pan_gesture_recognizer.replace(Some(pan));
            }
        } else if let Some(recognizer) = self.ivars().pan_gesture_recognizer.take() {
            self.removeGestureRecognizer(&recognizer);
        }
    }

    pub(crate) fn recognize_doubletap_gesture(&self, should_recognize: bool) {
        let mtm = MainThreadMarker::from(self);
        if should_recognize {
            if self.ivars().doubletap_gesture_recognizer.borrow().is_none() {
                let tap = unsafe {
                    UITapGestureRecognizer::initWithTarget_action(
                        mtm.alloc(),
                        Some(self),
                        Some(sel!(doubleTapGesture:)),
                    )
                };
                tap.setDelegate(Some(ProtocolObject::from_ref(self)));
                tap.setNumberOfTapsRequired(2);
                tap.setNumberOfTouchesRequired(1);
                self.addGestureRecognizer(&tap);
                self.ivars().doubletap_gesture_recognizer.replace(Some(tap));
            }
        } else if let Some(recognizer) = self.ivars().doubletap_gesture_recognizer.take() {
            self.removeGestureRecognizer(&recognizer);
        }
    }

    pub(crate) fn recognize_rotation_gesture(&self, should_recognize: bool) {
        let mtm = MainThreadMarker::from(self);
        if should_recognize {
            if self.ivars().rotation_gesture_recognizer.borrow().is_none() {
                let rotation = unsafe {
                    UIRotationGestureRecognizer::initWithTarget_action(
                        mtm.alloc(),
                        Some(self),
                        Some(sel!(rotationGesture:)),
                    )
                };
                rotation.setDelegate(Some(ProtocolObject::from_ref(self)));
                self.addGestureRecognizer(&rotation);
                self.ivars().rotation_gesture_recognizer.replace(Some(rotation));
            }
        } else if let Some(recognizer) = self.ivars().rotation_gesture_recognizer.take() {
            self.removeGestureRecognizer(&recognizer);
        }
    }

    fn handle_touches(&self, touches: &NSSet<UITouch>) {
        let window = self.window().unwrap();
        let mut touch_events = Vec::new();
        let os_supports_force = app_state::os_capabilities().force_touch;
        for touch in touches {
            let logical_location = touch.locationInView(None);
            let touch_type = touch.r#type();
            let force = if os_supports_force {
                let trait_collection = self.traitCollection();
                let touch_capability = trait_collection.forceTouchCapability();
                // Both the OS _and_ the device need to be checked for force touch support.
                if touch_capability == UIForceTouchCapability::Available
                    || touch_type == UITouchType::Pencil
                {
                    let force = touch.force();
                    let max_possible_force = touch.maximumPossibleForce();
                    let altitude_angle: Option<f64> = if touch_type == UITouchType::Pencil {
                        let angle = touch.altitudeAngle();
                        Some(angle as _)
                    } else {
                        None
                    };
                    Some(Force::Calibrated {
                        force: force as _,
                        max_possible_force: max_possible_force as _,
                        altitude_angle,
                    })
                } else {
                    None
                }
            } else {
                None
            };
            let touch_id = touch as *const UITouch as u64;
            let phase = touch.phase();
            let phase = match phase {
                UITouchPhase::Began => TouchPhase::Started,
                UITouchPhase::Moved => TouchPhase::Moved,
                // 2 is UITouchPhase::Stationary and is not expected here
                UITouchPhase::Ended => TouchPhase::Ended,
                UITouchPhase::Cancelled => TouchPhase::Cancelled,
                _ => panic!("unexpected touch phase: {phase:?}"),
            };

            let physical_location = {
                let scale_factor = self.contentScaleFactor();
                PhysicalPosition::from_logical::<(f64, f64), f64>(
                    (logical_location.x as _, logical_location.y as _),
                    scale_factor as f64,
                )
            };
            touch_events.push(EventWrapper::StaticEvent(Event::WindowEvent {
                window_id: RootWindowId(window.id()),
                event: WindowEvent::Touch(Touch {
                    device_id: DEVICE_ID,
                    id: touch_id,
                    location: physical_location,
                    force,
                    phase,
                }),
            }));
        }
        let mtm = MainThreadMarker::new().unwrap();
        app_state::handle_nonuser_events(mtm, touch_events);
    }

    fn handle_insert_text(&self, text: &NSString) {
        // `insertText:` replaces the marked text.
        self.discard_marked();
        self.type_text(&text.to_string());
        let window = self.window().unwrap();
        let window_id = RootWindowId(window.id());
        let mtm = MainThreadMarker::new().unwrap();
        // send individual events for each character
        app_state::handle_nonuser_events(
            mtm,
            text.to_string().chars().flat_map(|c| {
                let text = smol_str::SmolStr::from_iter([c]);
                // Emit both press and release events
                [ElementState::Pressed, ElementState::Released].map(|state| {
                    EventWrapper::StaticEvent(Event::WindowEvent {
                        window_id,
                        event: WindowEvent::KeyboardInput {
                            event: KeyEvent {
                                text: if state == ElementState::Pressed {
                                    Some(text.clone())
                                } else {
                                    None
                                },
                                state,
                                location: KeyLocation::Standard,
                                repeat: false,
                                logical_key: Key::Character(text.clone()),
                                physical_key: PhysicalKey::Unidentified(
                                    NativeKeyCode::Unidentified,
                                ),
                                platform_specific: KeyEventExtra {},
                            },
                            is_synthetic: false,
                            device_id: DEVICE_ID,
                        },
                    })
                })
            }),
        );
    }

    fn handle_delete_backward(&self) {
        // Backspace in a composition edits only the composition.
        if !self.ivars().ime.borrow().marked().is_empty() {
            self.ivars().ime.borrow_mut().delete_marked_backward();
            if self.ivars().ime.borrow().marked().is_empty() {
                self.discard_marked();
            } else {
                self.emit_preedit();
            }
            return;
        }
        self.ivars().ime.borrow_mut().delete_backward();
        let window = self.window().unwrap();
        let window_id = RootWindowId(window.id());
        let mtm = MainThreadMarker::new().unwrap();
        app_state::handle_nonuser_events(
            mtm,
            [ElementState::Pressed, ElementState::Released].map(|state| {
                EventWrapper::StaticEvent(Event::WindowEvent {
                    window_id,
                    event: WindowEvent::KeyboardInput {
                        device_id: DEVICE_ID,
                        event: KeyEvent {
                            state,
                            logical_key: Key::Named(NamedKey::Backspace),
                            physical_key: PhysicalKey::Code(KeyCode::Backspace),
                            platform_specific: KeyEventExtra {},
                            repeat: false,
                            location: KeyLocation::Standard,
                            text: None,
                        },
                        is_synthetic: false,
                    },
                })
            }),
        );
    }
}

impl WinitView {
    fn emit_ime_event(&self, ime: Ime) {
        self.emit_window_event(WindowEvent::Ime(ime));
    }

    fn emit_window_event(&self, event: WindowEvent) {
        let Some(window) = self.window() else {
            return;
        };
        let mtm = MainThreadMarker::new().unwrap();
        app_state::handle_nonuser_event(
            mtm,
            EventWrapper::StaticEvent(Event::WindowEvent {
                window_id: RootWindowId(window.id()),
                event,
            }),
        );
    }

    fn emit_preedit(&self) {
        let (text, selection) = {
            let doc = self.ivars().ime.borrow();
            (doc.marked().to_owned(), doc.marked_selection_bytes())
        };
        self.emit_ime_event(Ime::Preedit(text, Some(selection)));
    }

    /// Text reached the app at the caret.
    fn type_text(&self, text: &str) {
        if !self.ivars().ime_traits.get().secret {
            self.ivars().ime.borrow_mut().insert(text);
        }
    }

    /// Send `Ime::Commit`; `delete` is the bytes before the caret it replaces, for
    /// `take_ime_delete_surrounding`.
    fn queue_commit(&self, text: String, delete: Option<usize>) {
        {
            let mut deletes = self.ivars().ime_deletes.borrow_mut();
            // Bounded: an app that never takes them would grow it forever.
            if deletes.len() >= 32 {
                deletes.pop_front();
            }
            deletes.push_back(delete);
        }
        self.emit_ime_event(Ime::Commit(text));
    }

    pub(crate) fn take_ime_delete_surrounding(&self) -> Option<usize> {
        self.ivars().ime_deletes.borrow_mut().pop_front().flatten()
    }

    /// The composition is gone and nothing is committed.
    fn discard_marked(&self) {
        let marked = self.ivars().ime.borrow_mut().take_marked();
        if !marked.is_empty() {
            self.emit_ime_event(Ime::Preedit(String::new(), None));
        }
    }

    /// The composition becomes text.
    fn commit_marked(&self) {
        let marked = self.ivars().ime.borrow_mut().take_marked();
        if !marked.is_empty() {
            self.emit_ime_event(Ime::Preedit(String::new(), None));
            self.type_text(&marked);
            self.queue_commit(marked, None);
        }
    }

    /// Forget the text and drop the composition without committing it: the caret moved or the
    /// field changed. UIKit is told, so the keyboard stops composing and reads the document again.
    pub(crate) fn reset_ime(&self) {
        if self.ivars().ime.borrow().is_empty() {
            return;
        }
        let had_marked = !self.ivars().ime.borrow().marked().is_empty();
        self.changing_text(|| self.ivars().ime.borrow_mut().reset());
        if had_marked {
            self.emit_ime_event(Ime::Preedit(String::new(), None));
        }
    }

    pub(crate) fn reset_ime_with_request(&self, request_id: u64) {
        if !self.ivars().ime_allowed.get() {
            return;
        }
        self.reset_ime();
        // Same queue as the preedit above, so the fence comes after it.
        self.emit_window_event(WindowEvent::ImeResetCompleted { request_id });
    }

    pub(crate) fn set_ime_allowed(&self, allowed: bool) {
        if self.ivars().ime_allowed.replace(allowed) != allowed {
            self.reset_ime();
        }
    }

    /// A field with another purpose took the focus (or the field changed its kind): hand UIKit the new traits. `reloadInputViews`
    /// only when the traits really change, so a tap in the focused field leaves the keyboard alone.
    pub(crate) fn set_ime_purpose(&self, purpose: ImePurpose) {
        let new = traits(purpose);
        let old = self.ivars().ime_traits.replace(new);
        if old != new {
            nslog(&format!("gnobbi ime traits: {purpose:?} -> {new:?}"));
            self.sync_autofill_fields(new.credential.is_some());
            self.reset_ime();
            if self.ivars().ime_allowed.get() && old.credential != new.credential {
                // Another view takes the input (the keyboard stays up, UIKit reads its traits).
                self.become_input_responder();
            } else if new.credential.is_none() && unsafe { self.isFirstResponder() } {
                unsafe { self.reloadInputViews() };
            }
        }
    }

    pub(crate) fn set_ime_area(&self, area: CGRect) {
        self.ivars().ime_area.set(area);
    }

    /// Where the keyboard puts its candidates: the cursor area, or the whole view before one is set.
    fn ime_rect(&self) -> CGRect {
        let area = self.ivars().ime_area.get();
        if area.size.width > 0.0 || area.size.height > 0.0 {
            area
        } else {
            unsafe { msg_send![self, bounds] }
        }
    }

    /// `position` moved by `offset` characters (UTF-16 units), if it stays in the document.
    fn moved_position(&self, position: &UITextPosition, offset: i64) -> Option<Retained<UITextPosition>> {
        let target = downcast_position(position)?.offset().saturating_add(offset);
        let (start, end) = {
            let doc = self.ivars().ime.borrow();
            (doc.start() as i64, doc.end() as i64)
        };
        let mtm = MainThreadMarker::new().unwrap();
        (start..=end)
            .contains(&target)
            .then(|| Retained::into_super(WinitTextPosition::new(mtm, target)))
    }

    /// Run an app-side change of the text, bracketed by the notifications UIKit expects.
    fn changing_text(&self, change: impl FnOnce()) {
        let delegate = self.ivars().ime_delegate.borrow().as_deref().map(retained);
        let this: &ProtocolObject<dyn UITextInput> = ProtocolObject::from_ref(self);
        if let Some(delegate) = &delegate {
            unsafe {
                delegate.selectionWillChange(Some(this));
                delegate.textWillChange(Some(this));
            }
        }
        change();
        if let Some(delegate) = &delegate {
            unsafe {
                delegate.textDidChange(Some(this));
                delegate.selectionDidChange(Some(this));
            }
        }
    }
}

fn retained<T: objc2::Message>(object: &T) -> Retained<T> {
    // SAFETY: a reference is a live object, so retaining it can't fail.
    unsafe { Retained::retain(object as *const T as *mut T) }.unwrap()
}
