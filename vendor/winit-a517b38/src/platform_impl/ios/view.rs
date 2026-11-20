#![allow(clippy::unnecessary_cast)]
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{declare_class, msg_send, msg_send_id, mutability, sel, ClassType, DeclaredClass};
use objc2_foundation::{
    CGFloat, CGPoint, CGRect, CGSize, MainThreadMarker, NSArray, NSAttributedStringKey,
    NSComparisonResult, NSDictionary, NSInteger, NSObject, NSRange, NSSet, NSString,
};
use objc2_ui_kit::{
    UICoordinateSpace, UIEvent, UIForceTouchCapability, UIGestureRecognizer,
    UIGestureRecognizerDelegate, UIGestureRecognizerState, UIKeyInput, UIPanGestureRecognizer,
    UIPinchGestureRecognizer, UIResponder, UIRotationGestureRecognizer, UITapGestureRecognizer,
    UITextAutocorrectionType, UITextInput, UITextInputDelegate, UITextInputStringTokenizer,
    UITextInputTokenizer, UITextInputTraits, UITextLayoutDirection, UITextPosition, UITextRange,
    UITextSelectionRect, UITextSpellCheckingType, UITouch, UITouchPhase, UITouchType,
    UITraitEnvironment, UIView,
};

use super::app_state::{self, EventWrapper};
use super::ime::{downcast_position, downcast_range, WinitTextPosition, WinitTextRange};
use super::ime_document::ImeDocument;
use super::window::WinitUIWindow;
use crate::dpi::PhysicalPosition;
use crate::event::{ElementState, Event, Force, Ime, KeyEvent, Touch, TouchPhase, WindowEvent};
use crate::keyboard::{Key, KeyCode, KeyLocation, NamedKey, NativeKeyCode, PhysicalKey};
use crate::platform_impl::platform::DEVICE_ID;
use crate::platform_impl::KeyEventExtra;
use crate::window::{WindowAttributes, WindowId as RootWindowId};

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
    // Password fields: no suggestions, and the typed text isn't shown to UIKit.
    ime_secure: Cell<bool>,
    // The cursor area from `Window::set_ime_cursor_area`, in points; empty until set.
    ime_area: Cell<CGRect>,
    ime_delegate: RefCell<Option<Retained<ProtocolObject<dyn UITextInputDelegate>>>>,
    // One entry per `Ime::Commit` sent: the bytes before the caret it replaces.
    ime_deletes: RefCell<VecDeque<Option<usize>>>,
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
        #[method(isSecureTextEntry)]
        fn is_secure_text_entry(&self) -> bool {
            self.ivars().ime_secure.get()
        }

        #[method(autocorrectionType)]
        fn autocorrection_type(&self) -> UITextAutocorrectionType {
            if self.ivars().ime_secure.get() {
                UITextAutocorrectionType::No
            } else {
                UITextAutocorrectionType::Default
            }
        }

        #[method(spellCheckingType)]
        fn spell_checking_type(&self) -> UITextSpellCheckingType {
            if self.ivars().ime_secure.get() {
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
                let text = if self.ivars().ime_secure.get() {
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

impl WinitView {
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
            ime_secure: Cell::new(false),
            ime_area: Cell::new(CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: 0.0, height: 0.0 } }),
            ime_delegate: RefCell::new(None),
            ime_deletes: RefCell::new(VecDeque::new()),
        });
        let this: Retained<Self> = unsafe { msg_send_id![super(this), initWithFrame: frame] };

        this.setMultipleTouchEnabled(true);

        if let Some(scale_factor) = window_attributes.platform_specific.scale_factor {
            this.setContentScaleFactor(scale_factor as _);
        }

        this
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
        if !self.ivars().ime_secure.get() {
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

    pub(crate) fn set_ime_secure(&self, secure: bool) {
        if self.ivars().ime_secure.replace(secure) != secure {
            self.reset_ime();
            unsafe {
                if self.isFirstResponder() {
                    self.reloadInputViews();
                }
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
