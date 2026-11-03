import Foundation
import Security

/// Per-account database key kept in the Keychain; the Rust side only asks for it through this callback.
final class KeychainSecrets: SecretStore {
    private func query(_ account: UInt64) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "chat.gnobbi.mobile.store",
         kSecAttrAccount as String: String(account)]
    }
    func create(account: UInt64) throws -> String {
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else { throw MobileError.Storage }
        let secret = Data(bytes).base64EncodedString()
        var item = query(account)
        SecItemDelete(item as CFDictionary)
        item[kSecValueData as String] = Data(secret.utf8)
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        guard SecItemAdd(item as CFDictionary, nil) == errSecSuccess else { throw MobileError.Storage }
        return secret
    }
    func read(account: UInt64) throws -> String {
        var item = query(account)
        item[kSecReturnData as String] = true
        var out: AnyObject?
        guard SecItemCopyMatching(item as CFDictionary, &out) == errSecSuccess, let data = out as? Data,
              let text = String(data: data, encoding: .utf8) else { throw MobileError.Storage }
        return text
    }
}

/// Bridge callbacks arrive on the Rust events thread; hop to the main actor before touching state.
@MainActor
final class Session: ObservableObject {
    @Published var rooms: [RoomItem] = []
    @Published var state: SyncState = .connecting
    @Published var lastSend = ""
    @Published var error = ""
    @Published var signedIn = false
    private var handle: MobileSession?
    private let secrets = KeychainSecrets()
    private let dir: String = {
        // The bridge creates this directory itself with mode 0700 and refuses a pre-made looser one.
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        return base.appendingPathComponent("gnobbi").path
    }()

    func signIn(homeserver: String, user: String, password: String, trustedRootPem: String?) async {
        await open { listener in
            try await GnobbiMobile.signIn(dataDir: self.dir, homeserver: homeserver, userId: user, password: password,
                                    secrets: self.secrets, listener: listener, trustedRootPem: trustedRootPem)
        }
    }

    func restore(trustedRootPem: String?) async {
        await open { listener in
            try await restoreSession(dataDir: self.dir, secrets: self.secrets, listener: listener, trustedRootPem: trustedRootPem)
        }
    }

    private func open(_ make: (Listener) async throws -> MobileSession) async {
        error = ""
        do {
            handle = try await make(Listener(self))
            signedIn = true
        } catch MobileError.NoSession {
            // First launch: stay on the sign-in form.
        } catch {
            self.error = "Accesso non riuscito: \(error)"
        }
    }

    func send(room: RoomItem, text: String) {
        do { _ = try handle?.sendText(roomId: room.roomId, body: text); lastSend = "Invio in corso…" }
        catch { lastSend = "Invio non riuscito: \(error)" }
    }

    final class Listener: SyncListener {
        weak var session: Session?
        init(_ session: Session) { self.session = session }
        func onStatus(state: SyncState) { Task { @MainActor in self.session?.state = state } }
        func onRooms(rooms: [RoomItem]) { Task { @MainActor in self.session?.rooms = rooms } }
        func onSendResult(roomId: String, transactionId: String, error: MobileError?) {
            Task { @MainActor in self.session?.lastSend = error.map { "Invio non riuscito: \($0)" } ?? "Messaggio inviato" }
        }
    }
}
