import Foundation

/// Bridge callbacks arrive on the Rust events thread; hop to the main actor before touching state.
@MainActor
final class Session: ObservableObject {
    @Published var rooms: [RoomItem] = []
    @Published var state: SyncState = .connecting
    @Published var lastSend = ""
    @Published var error = ""
    @Published var signedIn = false
    @Published var loggingOut = false
    private var handle: MobileSession?
    private var trustedRootPem: String?
    private let dir: String = {
        // The bridge creates this directory itself with mode 0700 and refuses a pre-made looser one.
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        return base.appendingPathComponent("gnobbi").path
    }()

    func signIn(homeserver: String, user: String, password: String, trustedRootPem: String?) async {
        self.trustedRootPem = trustedRootPem
        await open { listener in
            try await GnobbiMobile.signIn(dataDir: self.dir, homeserver: homeserver, userId: user, password: password,
                                    secrets: nil, listener: listener, trustedRootPem: trustedRootPem)
        }
    }

    func restore(trustedRootPem: String?) async {
        self.trustedRootPem = trustedRootPem
        await open { listener in try await Self.restoreFirstValid(dir: self.dir, listener: listener, trustedRootPem: trustedRootPem) }
    }

    /// Native Keychain vault (secrets: nil). Tries each saved account; one the server rejects is stale
    /// (revoked token): forget it locally and move on. `NoSession` when none is left.
    nonisolated static func restoreFirstValid(dir: String, listener: SyncListener, trustedRootPem: String?) async throws -> MobileSession {
        for account in try listSavedAccounts(dataDir: dir) {
            do {
                return try await restoreSession(dataDir: dir, account: account, secrets: nil, listener: listener, trustedRootPem: trustedRootPem)
            } catch MobileError.Rejected {
                try removeLocalAccount(dataDir: dir, account: account, secrets: nil)
            } catch MobileError.SecretStore {
                continue // Keychain entry missing (restore from backup) or temporarily unavailable: skip, never delete
            }
        }
        throw MobileError.NoSession
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
    /// On failure (e.g. offline) the bridge has already stopped the worker, so the dead handle is dropped and the
    /// account is reopened with restore_session (the documented recovery); the error stays shown and "Esci" can be retried.
    func logout() async {
        guard let h = handle, !loggingOut else { return }
        loggingOut = true
        defer { loggingOut = false }
        error = ""
        do {
            try await Task.detached { try h.logout() }.value
            handle = nil
            rooms = []
            lastSend = ""
            signedIn = false
        } catch {
            handle = nil
            rooms = []
            signedIn = false
            let failure = "Uscita non riuscita: \(error)"
            await restore(trustedRootPem: trustedRootPem)
            self.error = failure
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
