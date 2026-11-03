import SwiftUI

struct RoomListView: View {
    @EnvironmentObject var session: Session
    @Environment(\.colorScheme) private var scheme
    @State private var target: RoomItem?
    @State private var autoSent = false

    var body: some View {
        let p = Palette.current(scheme)
        NavigationView {
            List(session.rooms.filter { !$0.isSpace }, id: \.roomId) { room in
                Button { target = room } label: {
                    HStack {
                        VStack(alignment: .leading, spacing: DesignTokens.spacingTextGap) {
                            Text(room.name).font(.headline).foregroundColor(p.text)
                            if let preview = room.preview { Text(preview).font(.subheadline).foregroundColor(p.secondary).lineLimit(1) }
                        }
                        Spacer()
                        if room.unread > 0 {
                            Text("\(room.unread)").font(.caption.bold()).padding(DesignTokens.spacingTextGap * 1.5)
                                .background(p.accent).foregroundColor(p.onAccent).clipShape(Capsule())
                        }
                    }.padding(.vertical, DesignTokens.spacingTextGap)
                }.listRowBackground(p.surface)
            }
            .listStyle(.plain)
            .navigationTitle("Chat")
            .toolbar {
                ToolbarItem(placement: .principal) { Text(label(session.state)).font(.caption).foregroundColor(p.secondary) }
                ToolbarItem(placement: .navigationBarTrailing) { Button("Esci") { Task { await session.logout() } }.disabled(session.loggingOut).foregroundColor(p.accent) }
            }
            .safeAreaInset(edge: .bottom) {
                if !session.error.isEmpty { Text(session.error).foregroundColor(p.error).padding() }
            }
        }
        .sheet(item: $target) { room in SendView(room: room) }
        .onChange(of: session.rooms.count) { _ in autoSend() }
    }

    private func label(_ s: SyncState) -> String {
        switch s {
        case .connecting: return "Connessione…"
        case .ready: return "Connesso"
        case .authenticationFailed: return "Accesso scaduto"
        case .syncFailed: return "Sincronizzazione non riuscita"
        }
    }

    /// Debug runs only: send once to the first room so a script can prove the send path.
    private func autoSend() {
        guard !autoSent, let text = TestEnv.autoSend, let room = session.rooms.first(where: { !$0.isSpace && !$0.invited }) else { return }
        autoSent = true
        session.send(room: room, text: text)
    }
}

extension RoomItem: Identifiable { public var id: String { roomId } }

struct SendView: View {
    @EnvironmentObject var session: Session
    @Environment(\.colorScheme) private var scheme
    @Environment(\.dismiss) private var dismiss
    let room: RoomItem
    @State private var text = ""
    @FocusState private var focused: Bool

    var body: some View {
        let p = Palette.current(scheme)
        VStack(alignment: .leading, spacing: DesignTokens.spacingRowPadY) {
            Text(room.name).font(.title3.bold()).foregroundColor(p.text)
            TextField("Messaggio", text: $text, axis: .vertical).focused($focused).padding(DesignTokens.spacingRowPadY)
                .overlay(RoundedRectangle(cornerRadius: DesignTokens.spacingTextGap).stroke(p.border)).submitLabel(.send).onSubmit(send)
            if !session.lastSend.isEmpty { Text(session.lastSend).foregroundColor(p.secondary) }
            HStack {
                Button("Chiudi") { dismiss() }
                Spacer()
                Button("Invia", action: send).disabled(text.trimmingCharacters(in: .whitespaces).isEmpty)
            }.foregroundColor(p.accent)
            Spacer()
        }
        .padding(DesignTokens.spacingRowPadY * 2).background(p.surface.ignoresSafeArea())
        .onAppear { focused = true }
    }

    private func send() { session.send(room: room, text: text); text = "" }
}
