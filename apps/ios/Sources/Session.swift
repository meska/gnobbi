import Foundation

/// Bridge callbacks arrive on the Rust events thread; hop to the main actor before touching state.
@MainActor
final class Session: ObservableObject {
    @Published var rooms: [RoomItem] = []
    @Published var state: SyncState = .connecting
    @Published var lastSend = ""
    @Published var error = ""
    @Published var signedIn = false
    private var handle: MobileSession?
    private let dir: String = {
        // The bridge creates this directory itself with mode 0700 and refuses a pre-made looser one.
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        return base.appendingPathComponent("gnobbi").path
    }()

    func signIn(homeserver: String, user: String, password: String, trustedRootPem: String?) async {
        await open { listener in
            try await GnobbiMobile.signIn(dataDir: self.dir, homeserver: homeserver, userId: user, password: password,
                                    secrets: nil, listener: listener, trustedRootPem: trustedRootPem)
        }
    }

    func restore(trustedRootPem: String?) async {
        await open { listener in
            // Native Keychain vault (secrets: nil). First saved account only: no account picker yet.
            guard let account = try listSavedAccounts(dataDir: self.dir).first else { throw MobileError.NoSession }
            return try await restoreSession(dataDir: self.dir, account: account, secrets: nil, listener: listener, trustedRootPem: trustedRootPem)
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

    /// Revokes the session on the server and deletes the Keychain entry and the account dir (blocking call, so off the main actor).
    /// On failure (e.g. offline) nothing is deleted and the user stays signed in with the error shown.
    func logout() async {
        guard let h = handle else { return }
        error = ""
        do {
            try await Task.detached { try h.logout() }.value
            handle = nil
            rooms = []
            lastSend = ""
            signedIn = false
        } catch {
            self.error = "Uscita non riuscita: \(error)"
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
