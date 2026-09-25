import Cocoa
import Darwin

// Writing an IPC reply to a client that already disconnected (timeout, ^C)
// raises SIGPIPE — default disposition kills the whole app. Ignore it and
// let write() surface EPIPE instead.
signal(SIGPIPE, SIG_IGN)

let app = NSApplication.shared
app.setActivationPolicy(.regular)
let delegate = AppDelegate()
app.delegate = delegate
app.activate(ignoringOtherApps: true)
app.run()
