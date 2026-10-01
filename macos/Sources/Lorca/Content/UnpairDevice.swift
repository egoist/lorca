import AppKit

/// The confirmation behind Unpair in the Devices pane. Another Device is unpaired by id; this one
/// forgets the identity, and the app goes back to onboarding.
@MainActor
enum UnpairDevice {
    static func confirm(_ device: Device, in window: NSWindow?) {
        let store = AppStore.shared
        // Only a Device that holds the identity pairs others, and only the backup phrase brings
        // the identity back once this one forgets it.
        let holdsIdentity = device.isThisDevice && store.isIdentityDevice
        let alert = NSAlert()
        if device.isThisDevice {
            alert.messageText = L("Unpair this computer?")
            alert.informativeText =
                holdsIdentity
                ? L("This computer forgets its keys, credentials, and synced chats, and bots assigned to it stop running until you assign them to another Runner. Your other paired Devices keep everything. Because this computer holds your identity, you need your backup phrase to use this account here again or to pair a new Device.")
                : L("This computer forgets its keys, credentials, and synced chats, and bots assigned to it stop running until you assign them to another Runner. Your other paired Devices keep everything, and you can pair again any time.")
        } else {
            alert.messageText = L("Unpair \"%@\"?", device.name)
            alert.informativeText =
                device.isRunner
                ? L("It loses its keys and synced chats the next time it connects, and bots assigned to it stop running until you assign them to another Runner. You can pair it again any time.")
                : L("It loses its keys and synced chats the next time it connects. You can pair it again any time.")
        }
        alert.alertStyle = holdsIdentity ? .critical : .warning
        alert.addButton(withTitle: L("Unpair"))
        alert.addButton(withTitle: L("Cancel"))
        alert.buttons.first?.hasDestructiveAction = true
        let run: (NSApplication.ModalResponse) -> Void = { response in
            guard response == .alertFirstButtonReturn else { return }
            Task { @MainActor in
                do {
                    if !device.isThisDevice {
                        try await store.unpairDevice(device.id)
                    } else if store.isMock {
                        // The demo has no CLI to forget the identity: onboarding opens over the demo
                        // account, as Show Onboarding Again opens it, and closing it brings the demo back.
                        NSApp.sendAction(#selector(AppDelegate.showOnboarding(_:)), to: nil, from: nil)
                    } else {
                        try await store.forgetIdentity()
                    }
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
