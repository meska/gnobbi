# Gnobbi roadmap

## Now
- **Android**: the same egui interface as desktop and iOS. Remaining: attachments and sharing, app icon, and end-to-end push notifications through a push gateway.
- **iOS**: continuous internal TestFlight builds. External testing waits for a stable build.
- **Repository cleanup** ahead of wider public use.
- **Share into Gnobbi** from other apps, Android first, then iOS.

## Next
- **First desktop release** (Linux, Windows, macOS), once the app is judged ready and the dependency licence audit passes.
- **Accessibility**: screen reader support for dialogs, drawers and account rows, tested with real screen readers.
- **Group settings**: banned members and unban, roles, room name and avatar, member details.
- **GIF / video / WebP library**, stored next to sticker packs, with muted looping video.
- **Matrix links**: an option to open `matrix:` links with Gnobbi.

## Later
- **Voice and video calls** using MatrixRTC, compatible with Element Call. A feasibility spike comes first.
- **Stickers**: space packs, a settings entry.

## Not planned
- In-app auto-update: updates come from app stores and GitHub Releases.
- Non-standard protocols or formats, and Element's integration manager or identity server.

## Recently shipped
- Unified chat list for all accounts, with a setting to switch it off; Unread and All chats count every account.
- Global message search over a local encrypted index; matching messages appear under the chat list.
- Send feedback from the account menu and Settings, with a diagnostics line prefilled.
- Animated GIF, WebP and APNG stickers and pictures; GIFs are sent animated.
- Avatar editor with pinch, twist and pan on touch screens.
- Touch lists: taps register on release, a fling-stopping touch is not a tap; long press on a chat opens the room menu.
- Room header bell for notification settings; message and chat menus with icons.
- Mention suggestions with member photos; emoji and sticker drawer in the composer.
- Member card with a large picture, opened from senders and mentions in the chat.
- Notifications show the sender name, the user ID and the message text.
- Timeline stays at the bottom when new messages or reactions arrive.
- Room info: copy button next to each room address; long addresses are shortened.
- Android: native Gradle shell, system bar and keyboard handling, TLS with bundled roots, push notifications with sender and text.
- Sticker packs: create and manage, picker with recents, favourites and search, editor.
- Reactions under messages, with counts and who reacted; touch message menu with quick reactions.
- Notifications cleared when a room is read; iOS badge shows total unread.
- Loading placeholders while a room opens or older history loads.
- Dialogs and the composer adapt to narrow screens and large text sizes.
- Crash report written at the next start after a panic.
- Desktop releases built on GitHub Actions; the updater reads the newest GitHub Release.

Last updated: 2026-11-26
