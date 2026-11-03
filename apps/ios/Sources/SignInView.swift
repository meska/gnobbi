import SwiftUI

struct SignInView: View {
    @EnvironmentObject var session: Session
    @Environment(\.colorScheme) private var scheme
    @State private var homeserver = "https://"
    @State private var user = ""
    @State private var password = ""
    @State private var busy = false
    @FocusState private var focus: Field?
    private enum Field { case homeserver, user, password }

    var body: some View {
        let p = Palette.current(scheme)
        VStack(alignment: .leading, spacing: DesignTokens.spacingRowPadY) {
            Text("Accedi").font(.title2.bold()).foregroundColor(p.text)
            field("Server", text: $homeserver, p).focused($focus, equals: .homeserver)
                .textInputAutocapitalization(.never).keyboardType(.URL).autocorrectionDisabled()
                .submitLabel(.next).onSubmit { focus = .user }
            field("Utente (@nome:server)", text: $user, p).focused($focus, equals: .user)
                .textInputAutocapitalization(.never).autocorrectionDisabled()
                .submitLabel(.next).onSubmit { focus = .password }
            SecureField("Password", text: $password).padding(DesignTokens.spacingRowPadY)
                .overlay(RoundedRectangle(cornerRadius: DesignTokens.spacingTextGap).stroke(p.border))
                .focused($focus, equals: .password).submitLabel(.go).onSubmit(go)
            if !session.error.isEmpty { Text(session.error).foregroundColor(p.error) }
            Button(action: go) {
                Text(busy ? "Accesso…" : "Accedi").frame(maxWidth: .infinity).padding(DesignTokens.spacingRowPadY)
                    .background(p.accent).foregroundColor(p.onAccent)
                    .clipShape(RoundedRectangle(cornerRadius: DesignTokens.spacingTextGap))
            }.disabled(busy || user.isEmpty || password.isEmpty)
            Spacer()
        }
        .padding(DesignTokens.spacingRowPadY * 2)
    }

    private func field(_ title: String, text: Binding<String>, _ p: Palette) -> some View {
        TextField(title, text: text).padding(DesignTokens.spacingRowPadY)
            .overlay(RoundedRectangle(cornerRadius: DesignTokens.spacingTextGap).stroke(p.border))
    }

    private func go() {
        busy = true
        Task {
            await session.signIn(homeserver: homeserver, user: user, password: password, trustedRootPem: TestEnv.caPem)
            busy = false
        }
    }
}
