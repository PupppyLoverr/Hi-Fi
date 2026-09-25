import Cocoa
import SwiftUI
import HiFiCore


/// hifi://newtab — command surface: URL/search field, quick actions, recents.
@MainActor
final class NewTabPaneView: PaneHostView {
    private let hosting: NSHostingView<NewTabView>
    override var firstResponderView: NSView { self }

    init(tab: HiFiCore.Tab, store: BrowserStore) {
        hosting = NSHostingView(rootView: NewTabView(store: store))
        super.init(frame: .zero)
        hosting.translatesAutoresizingMaskIntoConstraints = false
        addSubview(hosting)
        NSLayoutConstraint.activate([
            hosting.leadingAnchor.constraint(equalTo: leadingAnchor),
            hosting.trailingAnchor.constraint(equalTo: trailingAnchor),
            hosting.topAnchor.constraint(equalTo: topAnchor),
            hosting.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError() }
}

struct NewTabView: View {
    @ObservedObject var store: BrowserStore
    @State private var query = ""

    var body: some View {
        VStack(spacing: 18) {
            Spacer()
            Text("Hi-Fi").font(.system(size: 34, weight: .bold, design: .rounded))
                .foregroundStyle(.secondary)
            HStack {
                Image(systemName: "magnifyingglass").foregroundStyle(.tertiary)
                TextField("Search or enter address", text: $query, onCommit: open)
                    .textFieldStyle(.plain)
                    .font(.system(size: 16))
            }
            .padding(.horizontal, 14).padding(.vertical, 12)
            .background(RoundedRectangle(cornerRadius: 10).fill(Color.secondary.opacity(0.12)))
            .frame(maxWidth: 460)

            HStack(spacing: 10) {
                ActionChip("Terminal", icon: "terminal") { _ = store.openTab("hifi://terminal") }
                ActionChip("Agent", icon: "sparkles") { _ = store.openTab("hifi://agent") }
                ActionChip("Diff", icon: "doc.text.magnifyingglass") { _ = store.openTab("hifi://diff") }
                ActionChip("Split right", icon: "rectangle.split.2x1") {
                    if let id = store.focusedTabID { _ = store.splitTab(anchorID: id, side: .right) }
                }
            }

            let recents = store.historyEntries(matching: "", limit: 7)
            if !recents.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Recent").font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(.secondary)
                    ForEach(recents, id: \.url) { h in
                        Button { _ = store.openTab(h.url) } label: {
                            HStack {
                                Text(h.title.isEmpty ? h.url : h.title)
                                    .font(.system(size: 12)).lineLimit(1)
                                Spacer()
                                Text(h.url).font(.system(size: 10))
                                    .foregroundStyle(.tertiary).lineLimit(1)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
                .frame(maxWidth: 460)
                .padding(.top, 10)
            }
            Spacer()
            Text("⌘T command bar  ·  ⌘L address  ·  ⌘\\ sidebar  ·  ⌘⏎ split")
                .font(.system(size: 10)).foregroundStyle(.tertiary)
                .padding(.bottom, 18)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func open() {
        guard !query.isEmpty else { return }
        if let id = store.focusedTabID {
            // navigate within this newtab pane: replace it with the URL
            _ = store.openTab(query)
        } else {
            _ = store.openTab(query)
        }
    }
}

struct ActionChip: View {
    let label: String
    let icon: String
    let action: () -> Void
    init(_ label: String, icon: String, action: @escaping () -> Void) {
        self.label = label; self.icon = icon; self.action = action
    }
    var body: some View {
        Button { action() } label: {
            Label(label, systemImage: icon).font(.system(size: 11))
                .padding(.horizontal, 10).padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: 7)
                    .fill(Color.secondary.opacity(0.12)))
        }
        .buttonStyle(.plain)
    }
}

/// hifi://settings
@MainActor
final class SettingsPaneView: PaneHostView {
    init(tab: HiFiCore.Tab, store: BrowserStore) {
        super.init(frame: .zero)
        let hosting = NSHostingView(rootView: SettingsView(store: store))
        hosting.translatesAutoresizingMaskIntoConstraints = false
        addSubview(hosting)
        NSLayoutConstraint.activate([
            hosting.leadingAnchor.constraint(equalTo: leadingAnchor),
            hosting.trailingAnchor.constraint(equalTo: trailingAnchor),
            hosting.topAnchor.constraint(equalTo: topAnchor),
            hosting.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError() }
}

struct SettingsView: View {
    @ObservedObject var store: BrowserStore
    @State private var cliStatus = ""

    var body: some View {
        Form {
            Section("Hi-Fi") {
                LabeledContent("Version", value: "0.1.0")
                LabeledContent("Data", value: HiFiPaths.supportDir.path)
                LabeledContent("Worktrees", value: HiFiPaths.worktreeRoot.path)
            }
            Section("Spaces") {
                ForEach(store.state.spaces) { s in
                    HStack {
                        Circle().fill(Color(hex: s.accent) ?? .accentColor)
                            .frame(width: 8, height: 8)
                        Text(s.name)
                        Spacer()
                        Text(s.profileID.flatMap { p in
                            store.state.profiles.first(where: { $0.id == p })?.name
                        } ?? "default profile")
                        .foregroundStyle(.secondary).font(.system(size: 11))
                    }
                }
            }
            Section("Profiles") {
                ForEach(store.state.profiles) { p in
                    Text(p.name)
                }
                Button("Add profile") {
                    store.apply { st in
                        st.profiles.append(Profile(name: "Profile \(st.profiles.count + 1)"))
                    }
                }
            }
            Section("CLI") {
                Button("Install `hifi` into /usr/local/bin") { installCLI() }
                if !cliStatus.isEmpty { Text(cliStatus).font(.system(size: 11)) }
            }
            Section("Privacy") {
                Text("Local-first. Cookies, history and sessions stay on disk. Analytics off.")
                    .font(.system(size: 11)).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 480)
    }

    private func installCLI() {
        // prefer CLI sitting next to the app executable (bundled by bundle.sh)
        let appDir = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS")
        let cli = appDir.appendingPathComponent("hifi")
        let link = URL(fileURLWithPath: "/usr/local/bin/hifi")
        do {
            if FileManager.default.fileExists(atPath: link.path) {
                try FileManager.default.removeItem(at: link)
            }
            let src = FileManager.default.fileExists(atPath: cli.path)
                ? cli.path : ProcessInfo.processInfo.environment["HIFI_CLI_PATH"] ?? ""
            guard !src.isEmpty else { cliStatus = "hifi binary not found next to HiFi"; return }
            try FileManager.default.createSymbolicLink(atPath: link.path,
                                                       withDestinationPath: src)
            cliStatus = "installed: \(link.path) -> \(src)"
        } catch {
            cliStatus = "failed: \(error.localizedDescription)"
        }
    }
}
