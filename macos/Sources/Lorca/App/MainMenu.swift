import AppKit

enum MainMenu {
    static func build() -> NSMenu {
        let main = NSMenu()
        main.addItem(submenu(appMenu(), title: "Lorca"))
        main.addItem(submenu(fileMenu(), title: "File"))
        main.addItem(submenu(editMenu(), title: "Edit"))
        main.addItem(submenu(viewMenu(), title: "View"))
        main.addItem(submenu(chatMenu(), title: "Chat"))

        let window = windowMenu()
        main.addItem(submenu(window, title: "Window"))
        NSApp.windowsMenu = window

        main.addItem(submenu(debugMenu(), title: "Debug"))

        let help = helpMenu()
        main.addItem(submenu(help, title: "Help"))
        NSApp.helpMenu = help

        return main
    }

    // MARK: - Builders

    private static func submenu(_ menu: NSMenu, title: String) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        menu.title = title
        item.submenu = menu
        return item
    }

    @discardableResult
    private static func add(
        _ menu: NSMenu,
        _ title: String,
        _ action: Selector?,
        _ key: String = "",
        modifiers: NSEvent.ModifierFlags = .command,
        target: AnyObject? = nil,
        tag: Int = 0
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.keyEquivalentModifierMask = key.isEmpty ? [] : modifiers
        item.target = target
        item.tag = tag
        menu.addItem(item)
        return item
    }

    private static func appMenu() -> NSMenu {
        let menu = NSMenu()
        add(menu, "About Lorca", #selector(NSApplication.orderFrontStandardAboutPanel(_:)))
        if Updater.isEnabled {
            add(menu, "Check for Updates…", #selector(AppDelegate.checkForUpdates(_:)))
        }
        menu.addItem(.separator())
        add(menu, "Settings…", #selector(AppDelegate.showSettings(_:)), ",")
        menu.addItem(.separator())

        let services = NSMenu(title: "Services")
        let servicesItem = NSMenuItem(title: "Services", action: nil, keyEquivalent: "")
        servicesItem.submenu = services
        menu.addItem(servicesItem)
        NSApp.servicesMenu = services

        menu.addItem(.separator())
        add(menu, "Hide Lorca", #selector(NSApplication.hide(_:)), "h")
        add(
            menu, "Hide Others", #selector(NSApplication.hideOtherApplications(_:)), "h",
            modifiers: [.command, .option])
        add(menu, "Show All", #selector(NSApplication.unhideAllApplications(_:)))
        menu.addItem(.separator())
        add(menu, "Quit Lorca", #selector(NSApplication.terminate(_:)), "q")
        return menu
    }

    private static func fileMenu() -> NSMenu {
        let menu = NSMenu()
        add(menu, "New Bot…", #selector(AppDelegate.newBot(_:)), "n")
        add(menu, "New Group Chat…", #selector(AppDelegate.newGroupChat(_:)), "n", modifiers: [.command, .shift])
        menu.addItem(.separator())
        add(menu, "Pair a Device…", #selector(AppDelegate.pairDevice(_:)), "p", modifiers: [.command, .shift])
        menu.addItem(.separator())
        add(menu, "Close Window", #selector(NSWindow.performClose(_:)), "w")
        return menu
    }

    private static func editMenu() -> NSMenu {
        let menu = NSMenu()
        add(menu, "Undo", Selector(("undo:")), "z")
        add(menu, "Redo", Selector(("redo:")), "z", modifiers: [.command, .shift])
        menu.addItem(.separator())
        add(menu, "Cut", #selector(NSText.cut(_:)), "x")
        add(menu, "Copy", #selector(NSText.copy(_:)), "c")
        add(menu, "Paste", #selector(NSText.paste(_:)), "v")
        add(
            menu, "Paste and Match Style", #selector(NSTextView.pasteAsPlainText(_:)), "v",
            modifiers: [.command, .option, .shift])
        add(menu, "Select All", #selector(NSText.selectAll(_:)), "a")
        menu.addItem(.separator())
        add(menu, "Find…", #selector(AppDelegate.find(_:)), "f")
        menu.addItem(.separator())

        let speech = NSMenu(title: "Speech")
        add(speech, "Start Speaking", #selector(NSTextView.startSpeaking(_:)))
        add(speech, "Stop Speaking", #selector(NSTextView.stopSpeaking(_:)))
        let speechItem = NSMenuItem(title: "Speech", action: nil, keyEquivalent: "")
        speechItem.submenu = speech
        menu.addItem(speechItem)
        return menu
    }

    private static func viewMenu() -> NSMenu {
        let menu = NSMenu()
        add(menu, "Toggle Sidebar", #selector(NSSplitViewController.toggleSidebar(_:)), "b")
        add(
            menu, "Toggle Inspector", #selector(RootSplitViewController.toggleInspector(_:)), "b",
            modifiers: [.command, .shift])
        menu.addItem(.separator())
        add(menu, "Scroll to Latest", #selector(ChatViewController.scrollToLatest(_:)), "j")
        menu.addItem(.separator())
        add(
            menu, "Enter Full Screen", #selector(NSWindow.toggleFullScreen(_:)), "f",
            modifiers: [.command, .control])
        return menu
    }

    private static func chatMenu() -> NSMenu {
        let menu = NSMenu()
        add(menu, "Add Bot…", #selector(RootSplitViewController.addBotToChat(_:)), "b", modifiers: [.command, .option])
        add(menu, "Rename Chat…", #selector(RootSplitViewController.renameChat(_:)), "r")
        add(menu, "Pin Chat", #selector(RootSplitViewController.togglePinChat(_:)), "p")
        menu.addItem(.separator())
        add(menu, "Stop Responding", #selector(ChatViewController.stopResponding(_:)), ".")
        menu.addItem(.separator())
        add(menu, "Delete Chat", #selector(RootSplitViewController.deleteChat(_:)), "\u{8}")

        // ⌘1–⌘9 open the first nine chats in the sidebar, which shows the numbers while ⌘ is held.
        for number in 1...9 {
            let item = add(
                menu, "Go to Chat \(number)", #selector(RootSplitViewController.goToChat(_:)), "\(number)",
                tag: number)
            item.isHidden = true
            item.allowsKeyEquivalentWhenHidden = true
        }
        return menu
    }

    private static func windowMenu() -> NSMenu {
        let menu = NSMenu()
        add(menu, "Minimize", #selector(NSWindow.performMiniaturize(_:)), "m")
        add(menu, "Zoom", #selector(NSWindow.performZoom(_:)))
        menu.addItem(.separator())
        add(menu, "Bring All to Front", #selector(NSApplication.arrangeInFront(_:)))
        return menu
    }

    private static func debugMenu() -> NSMenu {
        let menu = NSMenu()
        add(
            menu, "Simulate CLI Offline", #selector(AppDelegate.toggleCLIConnection(_:)), "d",
            modifiers: [.command, .control], tag: MenuTag.simulateOffline)
        add(menu, "Replay Mock Data", #selector(AppDelegate.resetMockData(_:)), tag: MenuTag.replayMock)
        menu.addItem(.separator())
        add(menu, "Show Onboarding", #selector(AppDelegate.showOnboarding(_:)))
        return menu
    }

    private static func helpMenu() -> NSMenu {
        let menu = NSMenu()
        add(menu, "Lorca Help", #selector(AppDelegate.showHelp(_:)), "?")
        add(menu, "Architecture Notes", #selector(AppDelegate.showArchitecture(_:)))
        return menu
    }
}

enum MenuTag {
    static let simulateOffline = 8001
    static let replayMock = 8002
}
