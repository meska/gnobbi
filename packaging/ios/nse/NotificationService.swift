// Thin shell of the notification service extension: reads the push payload, the account's Keychain secret and the App Group
// data dir, calls the Rust core (crates/gnobbi-nse) once, and fills the notification. Never logs text or keys.
import Foundation
import Intents
import Security
import UserNotifications
import os

private let group = "group.net.meskatech.gnobbi"
private let keychainService = "gnobbi.sdk-protection-v1"
private let log = Logger(subsystem: "net.meskatech.gnobbi.nse", category: "nse")

/// Shown when the message is encrypted and its key did not arrive in time. Other languages: English.
private var encryptedText: String {
    Locale.preferredLanguages.first?.hasPrefix("it") == true ? "Nuovo messaggio cifrato" : "New encrypted message"
}

final class NotificationService: UNNotificationServiceExtension {
    private let lock = NSLock()
    private var handler: ((UNNotificationContent) -> Void)?
    private var best: UNMutableNotificationContent?

    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        let content = (request.content.mutableCopy() as? UNMutableNotificationContent) ?? UNMutableNotificationContent()
        lock.lock(); handler = contentHandler; best = content; lock.unlock()
        let info = request.content.userInfo
        guard let tag = info["gnobbi_account"] as? String, let room = info["room_id"] as? String, let event = info["event_id"] as? String else {
            return finish(nil) // not ours / not a Matrix push: the server's generic text stays
        }
        DispatchQueue.global(qos: .userInitiated).async {
            let (reply, avatar) = Self.fetch(tag: tag, room: room, event: event)
            self.finish { Self.apply(reply, avatar: avatar, tag: tag, room: room, to: $0) }
        }
    }

    /// The system is about to kill the extension: the generic text (the server's `aps.alert`) goes out now.
    override func serviceExtensionTimeWillExpire() { finish(nil) }

    /// Delivers once; later calls (late reply after the deadline) do nothing. `edit` turns the best attempt into the final content.
    private func finish(_ edit: ((UNMutableNotificationContent) -> UNNotificationContent)?) {
        lock.lock()
        guard let handler, let best else { lock.unlock(); return }
        self.handler = nil
        let content = edit?(best) ?? best
        lock.unlock()
        log.debug("nse done, peak \(gnobbi_nse_peak_footprint() / 1024, privacy: .public) KB")
        handler(content)
    }

    /// nil = failure of any kind (no store, signed out, offline...): the generic text stays.
    /// The sender's picture (PNG/JPEG, a small thumbnail) comes along when the sender has one.
    static func fetch(tag: String, room: String, event: String) -> ([String: Any]?, Data?) {
        let parts = tag.split(separator: ":", maxSplits: 2).map(String.init)
        guard parts.count == 3,
              let secret = secret(account: "profile-id:\(parts[2]):account-\(parts[1])") ?? secret(account: "account-id:\(parts[1])"),
              let base = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)?
                .appendingPathComponent("Library/Application Support/Gnobbi").path else { return (nil, nil) }
        var out: UnsafeMutablePointer<CChar>?
        var picture: UnsafeMutablePointer<UInt8>?
        var pictureLen = 0
        guard gnobbi_nse_fetch(base, tag, secret, room, event, &out, &picture, &pictureLen) == 0, let out else { return (nil, nil) }
        defer { gnobbi_nse_free(out); gnobbi_nse_free_bytes(picture, pictureLen) }
        let avatar = picture.map { Data(bytes: $0, count: pictureLen) }
        return ((try? JSONSerialization.jsonObject(with: Data(String(cString: out).utf8))) as? [String: Any], avatar)
    }

    /// The store passphrase: the app's Keychain item, shared through the keychain access group (no group named: the
    /// entitlement lists exactly one). Readable after first unlock.
    private static func secret(account: String) -> String? {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: keychainService,
                                    kSecAttrAccount as String: account, kSecReturnData as String: true, kSecMatchLimit as String: kSecMatchLimitOne]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess, let data = item as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    static func apply(_ reply: [String: Any]?, avatar: Data?, tag: String, room: String, to content: UNMutableNotificationContent) -> UNNotificationContent {
        guard let reply, let kind = reply["kind"] as? String else { return content }
        switch kind {
        case "suppress":
            // Edits and redactions: empty content. It only hides the alert with the Apple entitlement
            // com.apple.developer.usernotifications.filtering; without it iOS shows the original alert.
            return UNNotificationContent()
        case "encrypted":
            content.body = encryptedText
        case "message", "other":
            // Title, subtitle and trimmed body come ready from the Rust formatter (`gnobbi_nse::shown`).
            if let title = reply["title"] as? String, !title.isEmpty { content.title = title }
            if let subtitle = reply["subtitle"] as? String, !subtitle.isEmpty { content.subtitle = subtitle }
            if kind == "message", let body = reply["body"] as? String { content.body = body }
        default:
            return content
        }
        content.threadIdentifier = room
        // Same key the app's own alerts carry: `<catalog>:<worker>:<room>`; the tap handler reads it.
        let ids = tag.split(separator: ":", maxSplits: 2)
        var user = content.userInfo
        user["gnobbi"] = "\(ids[0]):\(ids[1]):\(room)"
        content.userInfo = user
        // Communication Notification: the sender's picture instead of the app icon. Only with a picture; without one, or if
        // the system refuses the intent, the notification stays exactly as built above.
        guard kind != "encrypted", let avatar, !avatar.isEmpty else { return content }
        return communication(reply, avatar: avatar, room: room, content: content) ?? content
    }

    private static func communication(_ reply: [String: Any], avatar: Data, room: String, content: UNMutableNotificationContent) -> UNNotificationContent? {
        guard let title = reply["title"] as? String, !title.isEmpty else { return nil }
        let senderId = (reply["sender_id"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? title
        let sender = INPerson(personHandle: INPersonHandle(value: senderId, type: .unknown), nameComponents: nil, displayName: title,
                              image: INImage(imageData: avatar), contactIdentifier: nil, customIdentifier: senderId)
        let group = (reply["group"] as? String).flatMap { $0.isEmpty ? nil : $0 }
        let intent = INSendMessageIntent(recipients: nil, outgoingMessageType: .outgoingMessageText, content: nil,
                                         speakableGroupName: group.map { INSpeakableString(spokenPhrase: $0) },
                                         conversationIdentifier: group == nil ? nil : room, serviceName: nil, sender: sender, attachments: nil)
        // In a group iOS shows the group name itself: the body goes without the room-name prefix.
        if group != nil, let text = reply["text"] as? String { content.body = text }
        let interaction = INInteraction(intent: intent, response: nil)
        interaction.direction = .incoming
        interaction.donate(completion: nil) // best effort; the notification does not wait for it
        return try? content.updating(from: intent)
    }
}
