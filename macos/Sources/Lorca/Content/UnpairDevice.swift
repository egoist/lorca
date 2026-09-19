import AppKit

/// The one confirmation every device list shares: the sidebar's context menu, the Devices
/// settings pane, and the device page.
enum UnpairDevice {
    static func confirm(_ device: Device, in window: NSWindow?) {
        guard !device.isThisDevice else { NSSound.beep(); return }
        let alert = NSAlert()
        alert.messageText = L("Unpair \"%@\"?", device.name)
        alert.informativeText =
            device.isRunner
            ? L("It loses its keys and synced chats the next time it connects, and bots assigned to it stop running until you assign them to another Runner. You can pair it again any time.")
            : L("It loses its keys and synced chats the next time it connects. You can pair it again any time.")
        alert.alertStyle = .warning
        alert.addButton(withTitle: L("Unpair"))
        alert.addButton(withTitle: L("Cancel"))
        alert.buttons.first?.hasDestructiveAction = true
        let run: (NSApplication.ModalResponse) -> Void = { response in
            guard response == .alertFirstButtonReturn else { return }
            Task { @MainActor in
                do {
                    try await AppStore.shared.unpairDevice(device.id)
                } catch {
                    let failed = NSAlert()
                    failed.messageText = L("Couldn’t unpair %@", device.name)
                    failed.informativeText = error.localizedDescription
                    failed.addButton(withTitle: L("OK"))
                    if let window {
                        failed.beginSheetModal(for: window) { _ in }
                    } else {
                        failed.runModal()
                    }
                }
            }
        }
        if let window {
            alert.beginSheetModal(for: window, completionHandler: run)
        } else {
            run(alert.runModal())
        }
    }
}
