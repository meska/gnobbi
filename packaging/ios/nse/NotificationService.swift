// Thin shell of the notification service extension: reads the push payload, the account's Keychain secret and the App Group
// data dir, calls the Rust core (crates/gnobbi-nse) once, and fills the notification. Never logs text or keys.
import Foundation
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
            let reply = Self.fetch(tag: tag, room: room, event: event)
            self.finish { Self.apply(reply, tag: tag, room: room, to: $0) }
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
    static func fetch(tag: String, room: String, event: String) -> [String: Any]? {
        let parts = tag.split(separator: ":", maxSplits: 2).map(String.init)
        guard parts.count == 3, let secret = secret(account: "account-id:\(parts[1])"),
              let base = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)?
                .appendingPathComponent("Library/Application Support/Gnobbi").path else { return nil }
        var out: UnsafeMutablePointer<CChar>?
        guard gnobbi_nse_fetch(base, tag, secret, room, event, &out) == 0, let out else { return nil }
        defer { gnobbi_nse_free(out) }
        return (try? JSONSerialization.jsonObject(with: Data(String(cString: out).utf8))) as? [String: Any]
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

    static func apply(_ reply: [String: Any]?, tag: String, room: String, to content: UNMutableNotificationContent) -> UNNotificationContent {
        guard let reply, let kind = reply["kind"] as? String else { return content }
        switch kind {
        case "suppress":
            // Edits and redactions: empty content. It only hides the alert with the Apple entitlement
            // com.apple.developer.usernotifications.filtering; without it iOS shows the original alert.
            return UNNotificationContent()
        case "encrypted":
            content.body = encryptedText
        case "message", "other":
            if let sender = reply["sender"] as? String, !sender.isEmpty { content.title = sender }
            if let room = reply["room"] as? String, !room.isEmpty { content.subtitle = room }
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
        return content
    }
}
