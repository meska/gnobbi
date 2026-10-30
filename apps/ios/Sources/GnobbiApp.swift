import SwiftUI

@main
struct GnobbiApp: App {
    @StateObject private var session = Session()
    var body: some Scene {
        WindowGroup {
            RootView().environmentObject(session)
        }
    }
}

/// In debug builds only, test servers are reached through an extra CA and credentials from the environment.
enum TestEnv {
    static var env: [String: String] { ProcessInfo.processInfo.environment }
    #if DEBUG
    static var caPem: String? {
        env["GNOBBI_TEST_CA"].flatMap { try? String(contentsOfFile: $0, encoding: .utf8) }
    }
    static var login: (String, String, String)? {
        guard let h = env["GNOBBI_TEST_HOMESERVER"], let u = env["GNOBBI_TEST_USER"], let p = env["GNOBBI_TEST_PASSWORD"] else { return nil }
        return (h, u, p)
    }
    static var autoSend: String? { env["GNOBBI_TEST_SEND"] }
    #else
    static var caPem: String? { nil }
    static var login: (String, String, String)? { nil }
    static var autoSend: String? { nil }
    #endif
}

struct RootView: View {
    @EnvironmentObject var session: Session
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        let p = Palette.current(scheme)
        Group {
            if session.signedIn { RoomListView() } else { SignInView() }
        }
        .background(p.surface.ignoresSafeArea())
        .task {
            await session.restore(trustedRootPem: TestEnv.caPem)
            if !session.signedIn, let (h, u, pw) = TestEnv.login {
                await session.signIn(homeserver: h, user: u, password: pw, trustedRootPem: TestEnv.caPem)
            }
        }
    }
}
