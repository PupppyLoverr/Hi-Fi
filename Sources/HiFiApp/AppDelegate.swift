import Cocoa
import HiFiCore

final class AppDelegate: NSObject, NSApplicationDelegate {

    private(set) var store: BrowserStore!
    private(set) var windowController: MainWindowController!
    private var ipcServer: IPCServer?

    func applicationDidFinishLaunching(_ notification: Notification) {
        try? HiFiPaths.ensureSupportDir()
        // Single instance: if another Hi-Fi already owns the IPC socket, exit.
        // The CLI's `open` activated the existing app; a second process would
        // unlink + rebind the socket and orphan the first.
        if UnixSocketClient.ping(path: HiFiPaths.socketPath) {
            HiFiLog.write("another instance owns the socket — exiting")
            exit(0)
        }
        buildMainMenu()

        store = BrowserStore()
        store.bootstrap()

        windowController = MainWindowController(store: store)
        store.windowController = windowController
        windowController.showWindow(nil)
        windowController.window?.makeKeyAndOrderFront(nil)

        ipcServer = IPCServer(store: store)
        do { try ipcServer?.start() }
        catch { HiFiLog.write("IPC server failed: \(error)") }

        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationWillTerminate(_ notification: Notification) {
        store?.persistNow()
        ipcServer?.stop()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    // MARK: - menu (standard editing keys + discoverability; shortcuts
    // themselves are routed through the window-level key monitor)

    private func buildMainMenu() {
        let main = NSMenu()

        let appItem = NSMenuItem()
        main.addItem(appItem)
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "About Hi-Fi", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Quit Hi-Fi", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu

        let fileItem = NSMenuItem(); main.addItem(fileItem)
        let file = NSMenu(title: "File")
        file.addItem(withTitle: "New Tab", action: #selector(MenuActions.newTab(_:)), keyEquivalent: "")
        file.addItem(withTitle: "Close Tab", action: #selector(MenuActions.closeTab(_:)), keyEquivalent: "")
        fileItem.submenu = file

        let editItem = NSMenuItem(); main.addItem(editItem)
        let edit = NSMenu(title: "Edit")
        edit.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        edit.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        edit.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "Z")
        editItem.submenu = edit

        let windowItem = NSMenuItem(); main.addItem(windowItem)
        let window = NSMenu(title: "Window")
        window.addItem(withTitle: "Minimize", action: #selector(NSWindow.miniaturize(_:)), keyEquivalent: "m")
        window.addItem(withTitle: "Zoom", action: #selector(NSWindow.zoom(_:)), keyEquivalent: "")
        windowItem.submenu = window
        NSApp.windowsMenu = window

        NSApp.mainMenu = main
    }

    @objc class MenuActions: NSObject {
        @objc func newTab(_ sender: Any?) {
            Task { @MainActor in
                (NSApp.delegate as? AppDelegate)?.windowController?.performKeyChord(.commandBar)
            }
        }
        @objc func closeTab(_ sender: Any?) {
            Task { @MainActor in
                (NSApp.delegate as? AppDelegate)?.windowController?.performKeyChord(.closeTab)
            }
        }
    }
}
