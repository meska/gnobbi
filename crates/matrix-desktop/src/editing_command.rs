//! Shared platform editing modifier for native fields and chat callers.
gpui::actions!(editing_command, [Copy, Paste, Cut, SelectAll]);

#[cfg(any(target_os = "macos", test))]
pub(crate) fn edit_menu() -> gpui::Menu {
    use gpui::{Menu, MenuItem, OsAction};
    Menu {
        name: "Edit".into(),
        items: vec![
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ],
    }
}

/// Native menu commands enter the existing editor command path, not OS input.
pub(crate) fn with_actions<E: gpui::InteractiveElement>(
    element: E,
    editable: bool,
    listener: impl Fn(&gpui::KeyDownEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> E {
    use std::rc::Rc;
    let listener = Rc::new(listener);
    let command = |chord: &'static str| {
        let listener = listener.clone();
        move |window: &mut gpui::Window, cx: &mut gpui::App| {
            listener(
                &gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke::parse(chord).expect("static editing command"),
                    is_held: false,
                },
                window,
                cx,
            );
        }
    };
    let copy = command("cmd-c");
    let paste = command("cmd-v");
    let cut = command("cmd-x");
    let select_all = command("cmd-a");
    let element = element
        .on_action(move |_: &Copy, window, cx| copy(window, cx))
        .on_action(move |_: &SelectAll, window, cx| select_all(window, cx));
    if editable {
        element
            .on_action(move |_: &Paste, window, cx| paste(window, cx))
            .on_action(move |_: &Cut, window, cx| cut(window, cx))
    } else {
        element
    }
}

/// Ctrl everywhere (unchanged); Cmd without Alt only on macOS.
pub(crate) fn command_modifier(modifiers: &gpui::Modifiers, mac: bool) -> bool {
    modifiers.control || (mac && modifiers.platform && !modifiers.alt)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::command_modifier;

    pub(crate) fn menu_action(index: usize) -> Box<dyn gpui::Action> {
        let menu = super::edit_menu();
        assert_eq!(menu.name.as_ref(), "Edit");
        assert_eq!(menu.items.len(), 4);
        let expected = [
            ("Cut", gpui::OsAction::Cut),
            ("Copy", gpui::OsAction::Copy),
            ("Paste", gpui::OsAction::Paste),
            ("Select All", gpui::OsAction::SelectAll),
        ];
        let gpui::MenuItem::Action {
            name,
            action,
            os_action,
        } = menu.items.into_iter().nth(index).unwrap()
        else {
            panic!("expected native editing action")
        };
        assert_eq!(name.as_ref(), expected[index].0);
        assert!(os_action == Some(expected[index].1));
        action
    }

    fn mods(chord: &str) -> gpui::Modifiers {
        gpui::Keystroke::parse(chord).unwrap().modifiers
    }

    #[test]
    fn ctrl_is_a_command_on_every_platform_and_cmd_only_on_macos() {
        for mac in [false, true] {
            assert!(command_modifier(&mods("ctrl-a"), mac));
            assert!(!command_modifier(&mods("a"), mac));
        }
        assert!(command_modifier(&mods("cmd-a"), true));
        assert!(!command_modifier(&mods("cmd-a"), false));
        assert!(!command_modifier(&mods("alt-cmd-a"), true));
    }
}
