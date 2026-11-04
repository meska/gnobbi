import Foundation
import Security
import XCTest
@testable import GnobbiMobile

/// Runs against the disposable Synapse/Tuwunel from apps/fixture (never a real account).
/// Env (TEST_RUNNER_ prefix for xcodebuild): GNOBBI_FIXTURE=fixture.json, GNOBBI_CA=ca.pem, GNOBBI_SERVER=synapse|tuwunel.
final class Rec: SyncListener {
    private let lock = NSLock()
    private var _rooms: [RoomItem] = []
    var rooms: [RoomItem] { lock.lock(); defer { lock.unlock() }; return _rooms }
    func onStatus(state: SyncState) {}
    func onRooms(rooms: [RoomItem]) { lock.lock(); _rooms = rooms; lock.unlock() }
    func onSendResult(roomId: String, transactionId: String, error: MobileError?) {}
}

final class TrustCA: NSObject, URLSessionDelegate {
    let anchor: SecCertificate
    init(pem: String) {
        let b64 = pem.split(separator: "\n").filter { !$0.hasPrefix("-----") }.joined()
        anchor = SecCertificateCreateWithData(nil, Data(base64Encoded: b64)! as CFData)!
    }
    func urlSession(_ s: URLSession, didReceive c: URLAuthenticationChallenge, completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
        guard let t = c.protectionSpace.serverTrust else { return completionHandler(.cancelAuthenticationChallenge, nil) }
        SecTrustSetAnchorCertificates(t, [anchor] as CFArray)
        SecTrustSetAnchorCertificatesOnly(t, true)
        completionHandler(.useCredential, URLCredential(trust: t))
    }
}

final class VaultTests: XCTestCase {
    static let service = "gnobbi.sdk-protection-v1"
    var dir = ""
    var ca = ""
    var url = ""
    var users: [(id: String, pw: String, token: String)] = []
    var open: [MobileSession] = []

    override func setUpWithError() throws {
        let e = ProcessInfo.processInfo.environment
        let f = try XCTUnwrap(e["GNOBBI_FIXTURE"], "GNOBBI_FIXTURE missing")
        let srv = try XCTUnwrap(e["GNOBBI_SERVER"])
        ca = try String(contentsOfFile: try XCTUnwrap(e["GNOBBI_CA"]), encoding: .utf8)
        let j = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: f))) as! [String: Any]
        let s = (j["servers"] as! [String: Any])[srv] as! [String: Any]
        url = s["url"] as! String
        users = (s["users"] as! [String: [String: String]]).sorted { $0.key < $1.key }.map { ($0.key, $0.value["password"]!, $0.value["token"]!) }
        XCTAssertEqual(users.count, 2)
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("vt-\(UUID().uuidString)").path
        try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        for a in try kcItems() { SecItemDelete(match(a) as CFDictionary) } // clean slate: this simulator only
    }

    override func tearDown() {
        for s in open { try? s.stop() }
        for a in (try? listSavedAccounts(dataDir: dir)) ?? [] { try? removeLocalAccount(dataDir: dir, account: a, secrets: nil) }
        try? FileManager.default.removeItem(atPath: dir)
    }

    // MARK: helpers
    func match(_ account: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: Self.service, kSecAttrAccount as String: account,
         kSecAttrSynchronizable as String: kSecAttrSynchronizableAny]
    }
    func kcItems() throws -> [String] {
        var out: CFTypeRef?
        let q: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: Self.service,
                                kSecReturnAttributes as String: true, kSecMatchLimit as String: kSecMatchLimitAll,
                                kSecAttrSynchronizable as String: kSecAttrSynchronizableAny]
        let st = SecItemCopyMatching(q as CFDictionary, &out)
        if st == errSecItemNotFound { return [] }
        XCTAssertEqual(st, errSecSuccess)
        return (out as! [[String: Any]]).map { $0[kSecAttrAccount as String] as! String }.sorted()
    }
    func attrs(_ account: String) -> [String: Any] {
        var out: CFTypeRef?
        var q = match(account); q[kSecReturnAttributes as String] = true; q[kSecReturnData as String] = true
        XCTAssertEqual(SecItemCopyMatching(q as CFDictionary, &out), errSecSuccess, "item \(account)")
        return out as? [String: Any] ?? [:]
    }
    func signInUser(_ i: Int, _ rec: Rec = Rec()) async throws -> (MobileSession, Rec) {
        let s = try await signIn(dataDir: dir, homeserver: url, userId: users[i].id, password: users[i].pw, secrets: nil, listener: rec, trustedRootPem: ca)
        open.append(s)
        return (s, rec)
    }
    func restore(_ id: UInt64, _ rec: Rec = Rec()) async throws -> (MobileSession, Rec) {
        let s = try await restoreSession(dataDir: dir, account: id, secrets: nil, listener: rec, trustedRootPem: ca)
        open.append(s)
        return (s, rec)
    }
    func close(_ s: MobileSession) { try? s.stop(); open.removeAll { $0.accountId() == s.accountId() } }
    func waitRooms(_ r: Rec, _ n: Int = 3) async throws {
        for _ in 0..<120 { if r.rooms.count >= n { return }; try await Task.sleep(nanoseconds: 500_000_000) }
        XCTFail("rooms: got \(r.rooms.count), want >= \(n)")
    }
    func logout(_ s: MobileSession) async throws { try await Task.detached { try s.logout() }.value; open.removeAll { $0.accountId() == s.accountId() } }
    func assertProtected(_ account: String, file: StaticString = #filePath, line: UInt = #line) {
        let a = attrs(account)
        XCTAssertEqual(a[kSecAttrAccessible as String] as? String, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String, "accessibility", file: file, line: line)
        // Absent or 0 both mean "not synchronizable"; 1 would sync through iCloud Keychain.
        XCTAssertNotEqual((a[kSecAttrSynchronizable as String] as? NSNumber)?.intValue, 1, "synchronizable", file: file, line: line)
        XCTAssertGreaterThan((a[kSecValueData as String] as? Data)?.count ?? 0, 0, file: file, line: line)
    }
    func serverLogoutAll(_ token: String) async throws {
        let s = URLSession(configuration: .ephemeral, delegate: TrustCA(pem: ca), delegateQueue: nil)
        var r = URLRequest(url: URL(string: url + "/_matrix/client/v3/logout/all")!)
        r.httpMethod = "POST"; r.httpBody = Data("{}".utf8)
        r.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization"); r.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let (_, resp) = try await s.data(for: r)
        XCTAssertEqual((resp as! HTTPURLResponse).statusCode, 200)
    }

    // MARK: 1. save, read (restore after "app close"), remove
    func testSaveRestoreLogout() async throws {
        XCTAssertEqual(try kcItems(), [])
        let (s, rec) = try await signInUser(0)
        try await waitRooms(rec)
        let items = try kcItems()
        XCTAssertEqual(items.count, 1, "one Keychain item after sign-in: \(items)")
        XCTAssertTrue(items[0].hasSuffix(":\(s.accountId())"), items[0])
        assertProtected(items[0])
        // "Close the app": stop the session and drop every handle; only disk + Keychain remain.
        let id = s.accountId(); close(s)
        XCTAssertEqual(try listSavedAccounts(dataDir: dir), [id])
        let (s2, rec2) = try await restore(id)
        try await waitRooms(rec2)
        XCTAssertTrue(rec2.rooms.contains { $0.name.hasPrefix("Stanza prova") }, "seeded rooms present after restore")
        try await logout(s2)
        XCTAssertEqual(try kcItems(), [], "Keychain item deleted by logout")
        XCTAssertEqual(try listSavedAccounts(dataDir: dir), [])
        _ = rec
    }

    // MARK: 2. two accounts at once: separate items, logout of one leaves the other
    func testTwoAccountsIsolated() async throws {
        let (a, ra) = try await signInUser(0)
        let (b, rb) = try await signInUser(1)
        XCTAssertNotEqual(a.accountId(), b.accountId())
        try await waitRooms(ra, 1); try await waitRooms(rb, 1)
        let items = try kcItems()
        XCTAssertEqual(items.count, 2, "\(items)")
        items.forEach { assertProtected($0) }
        let secrets = Set(items.map { attrs($0)[kSecValueData as String] as! Data })
        XCTAssertEqual(secrets.count, 2, "each account has its own secret")
        try await logout(a)
        let left = try kcItems()
        XCTAssertEqual(left.count, 1)
        XCTAssertTrue(left[0].hasSuffix(":\(b.accountId())"), "survivor is b: \(left)")
        // b still restores after a's removal
        let bid = b.accountId(); close(b)
        XCTAssertEqual(try listSavedAccounts(dataDir: dir), [bid])
        let (b2, rb2) = try await restore(bid)
        try await waitRooms(rb2, 1)
        try await logout(b2)
        XCTAssertEqual(try kcItems(), [])
    }

    // MARK: 3. edge cases
    func testMissingItem() async throws {
        // user b: Tuwunel rate-limits ~5 logins per account per 5 minutes, and the other tests spend user a's.
        let (s, _) = try await signInUser(1)
        let id = s.accountId(); close(s)
        for a in try kcItems() { XCTAssertEqual(SecItemDelete(match(a) as CFDictionary), errSecSuccess) }
        do { let (s2, _) = try await restore(id); close(s2); XCTFail("restore succeeded without the Keychain item") }
        catch { XCTAssertEqual(error as? MobileError, .SecretStore, "got \(error)") }
        try removeLocalAccount(dataDir: dir, account: id, secrets: nil)
        XCTAssertEqual(try listSavedAccounts(dataDir: dir), [])
    }

    func testCorruptItem() async throws {
        let (s, _) = try await signInUser(0)
        let id = s.accountId(); close(s)
        let acc = try kcItems()[0]
        let junk = Data((0..<32).map { _ in UInt8.random(in: 0...255) })
        XCTAssertEqual(SecItemUpdate(match(acc) as CFDictionary, [kSecValueData as String: junk] as CFDictionary), errSecSuccess)
        do { let (s2, _) = try await restore(id); close(s2); XCTFail("restore succeeded with a corrupt secret") }
        catch { print("CORRUPT-ITEM restore error: \(error)") ; XCTAssertTrue(error is MobileError) }
        try removeLocalAccount(dataDir: dir, account: id, secrets: nil)
        XCTAssertEqual(try kcItems(), [], "remove_local_account clears the corrupt item")
    }

    func testTokenInvalidatedOnServer() async throws {
        let (s, _) = try await signInUser(0)
        let id = s.accountId(); close(s)
        try await serverLogoutAll(users[0].token) // revokes every session of the user, the app's included
        do { let (s2, _) = try await restore(id); close(s2); XCTFail("restore succeeded with a revoked token") }
        catch { XCTAssertEqual(error as? MobileError, .Rejected, "got \(error)") }
        print("REVOKED items left: \(try kcItems())")
        try removeLocalAccount(dataDir: dir, account: id, secrets: nil)
        XCTAssertEqual(try kcItems(), [])
        // and the user can sign in again
        let (s3, r3) = try await signInUser(0)
        try await waitRooms(r3)
        try await logout(s3)
    }

    /// Session.restore opens listSavedAccounts().first. After a server-side revoke the stale account 1 stays saved
    /// (restore -> Rejected) and a fresh sign-in becomes account 2, so the next launch must not be stuck on account 1.
    func testRelaunchAfterRevokeAndResignin() async throws {
        let (s, _) = try await signInUser(1)
        close(s)
        try await serverLogoutAll(users[1].token)
        let (s2, r2) = try await signInUser(1) // user signs in again from the form
        try await waitRooms(r2, 1)
        close(s2)
        do {
            let s3 = try await Session.restoreFirstValid(dir: dir, listener: Rec(), trustedRootPem: ca)
            open.append(s3)
            XCTAssertEqual(s3.accountId(), s2.accountId())
            close(s3)
        } catch { XCTFail("next launch fails with \(error), although account \(s2.accountId()) is valid") }
        XCTAssertEqual(try listSavedAccounts(dataDir: dir), [s2.accountId()], "stale account forgotten")
    }

    /// iOS keeps Keychain items when the app is deleted, but the data dir goes away: the next install finds an
    /// orphan item for the account ID it is about to reuse (smallest free ID).
    func testReinstallLeavesOrphanKeychainItem() async throws {
        // The leftover of the previous install, planted directly (saves a login: Tuwunel rate-limits per account).
        var add = match("account-id:1"); add.removeValue(forKey: kSecAttrSynchronizable as String)
        add[kSecValueData as String] = Data("orphan-secret-from-previous-install".utf8)
        XCTAssertEqual(SecItemAdd(add as CFDictionary, nil), errSecSuccess)
        XCTAssertEqual(try listSavedAccounts(dataDir: dir), [])
        XCTAssertEqual(try kcItems().count, 1, "orphan item survives")
        let (s2, r2) = try await signInUser(1) // must succeed: the user just reinstalled the app
        try await waitRooms(r2, 1)
        try await logout(s2)
    }

    /// Restore from backup / new device: the data dir returns, the ThisDeviceOnly Keychain item does not. Account 1
    /// fails with SecretStore on every launch; it must be skipped (and kept: the Keychain may be only temporarily
    /// unavailable) so that the valid account 2 still opens.
    func testRestoreSkipsAccountWithoutKeychainItem() async throws {
        let (s, _) = try await signInUser(1)
        let stale = s.accountId(); close(s)
        for a in try kcItems() { XCTAssertEqual(SecItemDelete(match(a) as CFDictionary), errSecSuccess) }
        let (s2, r2) = try await signInUser(1)
        try await waitRooms(r2, 1)
        close(s2)
        let s3 = try await Session.restoreFirstValid(dir: dir, listener: Rec(), trustedRootPem: ca)
        open.append(s3)
        XCTAssertEqual(s3.accountId(), s2.accountId())
        close(s3)
        XCTAssertEqual(try listSavedAccounts(dataDir: dir), [stale, s2.accountId()], "stale dir kept")
    }
}
