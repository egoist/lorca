import AppKit

MainActor.assumeIsolated { StartupTrace.mark("main") }
let application = NSApplication.shared

// NSApplication holds its delegate weakly, so this global keeps it alive.
let appDelegate = MainActor.assumeIsolated { AppDelegate() }

application.delegate = appDelegate
application.setActivationPolicy(.regular)
application.run()
