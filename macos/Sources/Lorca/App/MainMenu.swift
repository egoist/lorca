import AppKit

enum MainMenu {
    static func build() -> NSMenu {
        let main = NSMenu()
        main.addItem(submenu(appMenu(), title: "Lorca"))
        main.addItem(submenu(fileMenu(), title: L("File")))
        main.addItem(submenu(editMenu(), title: L("Edit")))
        main.addItem(submenu(viewMenu(), title: L("View")))
        main.addItem(submenu(chatMenu(), title: L("Chat")))

        let window = windowMenu()
        main.addItem(submenu(window, title: L("Window")))
        NSApp.windowsMenu = window

        main.addItem(submenu(debugMenu(), title: "Debug"))

        let help = helpMenu()
        main.addItem(submenu(help, title: L("Help")))
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
        add(menu, L("About Lorca"), #selector(NSApplication.orderFrontStandardAboutPanel(_:)))
        if Updater.isEnabled {
            add(menu, L("Check for Updates…"), #selector(AppDelegate.checkForUpdates(_:)))
        }
        menu.addItem(.separator())
        add(menu, L("Settings…"), #selector(AppDelegate.showSettings(_:)), ",")
        menu.addItem(.separator())

        let services = NSMenu(title: L("Services"))
        let servicesItem = NSMenuItem(title: L("Services"), action: nil, keyEquivalent: "")
        servicesItem.submenu = services
        menu.addItem(servicesItem)
        NSApp.servicesMenu = services

        menu.addItem(.separator())
        add(menu, L("Hide Lorca"), #selector(NSApplication.hide(_:)), "h")
        add(
            menu, L("Hide Others"), #selector(NSApplication.hideOtherApplications(_:)), "h",
            modifiers: [.command, .option])
        add(menu, L("Show All"), #selector(NSApplication.unhideAllApplications(_:)))
        menu.addItem(.separator())
        add(menu, L("Quit Lorca"), #selector(NSApplication.terminate(_:)), "q")
        return menu
    }

    private static func fileMenu() -> NSMenu {
        let menu = NSMenu()
        add(menu, L("New Bot…"), #selector(AppDelegate.newBot(_:)), "n")
        add(menu, L("New Group Chat…"), #selector(AppDelegate.newGroupChat(_:)), "n", modifiers: [.command, .shift])
        menu.addItem(.separator())
        add(menu, L("Pair a Device…"), #selector(AppDelegate.pairDevice(_:)), "p", modifiers: [.command, .shift])
        menu.addItem(.separator())
        add(menu, L("Close Window"), #selector(NSWindow.performClose(_:)), "w")
        return menu
    }

    private static func editMenu() -> NSMenu {
        let menu = NSMenu()
        add(menu, L("Undo"), Selector(("undo:")), "z")
        add(menu, L("Redo"), Selector(("redo:")), "z", modifiers: [.command, .shift])
        menu.addItem(.separator())
        add(menu, L("Cut"), #selector(NSText.cut(_:)), "x")
        add(menu, L("Copy"), #selector(NSText.copy(_:)), "c")
        add(menu, L("Paste"), #selector(NSText.paste(_:)), "v")
        add(
            menu, L("Paste and Match Style"), #selector(NSTextView.pasteAsPlainText(_:)), "v",
            modifiers: [.command, .option, .shift])
        add(menu, L("Select All"), #selector(NSText.selectAll(_:)), "a")
        menu.addItem(.separator())
        add(menu, L("Find…"), #selector(AppDelegate.find(_:)), "f")
        menu.addItem(.separator())

        let speech = NSMenu(title: L("Speech"))
        add(speech, L("Start Speaking"), #selector(NSTextView.startSpeaking(_:)))
        add(speech, L("Stop Speaking"), #selector(NSTextView.stopSpeaking(_:)))
        let speechItem = NSMenuItem(title: L("Speech"), action: nil, keyEquivalent: "")
        speechItem.submenu = speech
        menu.addItem(speechItem)
        return menu
    }

    private static func viewMenu() -> NSMenu {
        let menu = NSMenu()
        add(menu, L("Command Palette…"), #selector(AppDelegate.toggleCommandPalette(_:)), "k")
        menu.addItem(.separator())
        add(menu, L("Toggle Sidebar"), #selector(NSSplitViewController.toggleSidebar(_:)), "b")
        add(
            menu, L("Toggle Inspector"), #selector(RootSplitViewController.toggleInspector(_:)), "b",
            modifiers: [.command, .shift])
        menu.addItem(.separator())
        add(menu, L("Scroll to Latest"), #selector(ChatViewController.scrollToLatest(_:)), "j")
        menu.addItem(.separator())
        add(
            menu, L("Enter Full Screen"), #selector(NSWindow.toggleFullScreen(_:)), "f",
            modifiers: [.command, .control])
        return menu
    }

    private static func chatMenu() -> NSMenu {
        let menu = NSMenu()
        add(menu, L("Add Bot…"), #selector(RootSplitViewController.addBotToChat(_:)), "b", modifiers: [.command, .option])
        add(menu, L("Rename Chat…"), #selector(RootSplitViewController.renameChat(_:)), "r")
        add(menu, L("Pin Chat"), #selector(RootSplitViewController.togglePinChat(_:)), "p")
        menu.addItem(.separator())
        add(menu, L("Stop Responding"), #selector(ChatViewController.stopResponding(_:)), ".")
        menu.addItem(.separator())
        add(menu, L("Delete Chat"), #selector(RootSplitViewController.deleteChat(_:)), "\u{8}")

        // ⌘1–⌘9 open the first nine chats in the sidebar, which shows the numbers while ⌘ is held.
        for number in 1...9 {
            let item = add(
                menu, L("Go to Chat %d", number), #selector(RootSplitViewController.goToChat(_:)), "\(number)",
                tag: number)
            item.isHidden = true
            item.allowsKeyEquivalentWhenHidden = true
        }
        return menu
    }

    private static func windowMenu() -> NSMenu {
        let menu = NSMenu()
        add(menu, L("Minimize"), #selector(NSWindow.performMiniaturize(_:)), "m")
        add(menu, L("Zoom"), #selector(NSWindow.performZoom(_:)))
        menu.addItem(.separator())
        add(menu, L("Bring All to Front"), #selector(NSApplication.arrangeInFront(_:)))
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
        add(menu, L("Lorca Help"), #selector(AppDelegate.showHelp(_:)), "?")
        add(menu, L("Architecture Notes"), #selector(AppDelegate.showArchitecture(_:)))
        return menu
    }
}

enum MenuTag {
    static let simulateOffline = 8001
    static let replayMock = 8002
}
