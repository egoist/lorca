import AppKit

/// The menu items that file and quiet a chat, shared by a sidebar row's menu and the Chat menu.
/// Their actions go down the responder chain to `RootSplitViewController`, which acts on the
/// chat on screen.
@MainActor
enum ChatMenus {
    /// How long Mute keeps a chat quiet, in seconds; zero is until unmuted.
    static let muteSpans: [(title: String, seconds: Int)] = [
        (L("For 1 Hour"), 3600),
        (L("For 8 Hours"), 8 * 3600),
        (L("For 1 Week"), 7 * 24 * 3600),
        (L("Always"), 0),
    ]

    /// Mute and its spans, or Unmute for a muted chat.
    static func muteItem(for chat: Chat) -> NSMenuItem {
        if chat.isMuted {
            return NSMenuItem(title: L("Unmute"), action: #selector(RootSplitViewController.unmuteChat(_:)), keyEquivalent: "")
        }
        let item = NSMenuItem(title: L("Mute"), action: nil, keyEquivalent: "")
        let spans = NSMenu(title: L("Mute"))
        fillMuteMenu(spans)
        item.submenu = spans
        return item
    }

    static func fillMuteMenu(_ menu: NSMenu) {
        menu.removeAllItems()
        for span in muteSpans {
            let item = NSMenuItem(title: span.title, action: #selector(RootSplitViewController.muteChat(_:)), keyEquivalent: "")
            item.tag = span.seconds
            menu.addItem(item)
        }
    }

    /// Move to Section and the sections, or Move to New Section… while there are none.
    static func sectionItem(for chat: Chat, store: AppStore) -> NSMenuItem {
        guard !store.sections.isEmpty else {
            return NSMenuItem(title: L("Move to New Section…"), action: #selector(RootSplitViewController.moveChatToNewSection(_:)), keyEquivalent: "")
        }
        let item = NSMenuItem(title: L("Move to Section"), action: nil, keyEquivalent: "")
        let sections = NSMenu(title: L("Move to Section"))
        fillSectionMenu(sections, for: chat, store: store)
        item.submenu = sections
        return item
    }

    /// Each section, checked where the chat is, then Chats for no section, then New Section….
    static func fillSectionMenu(_ menu: NSMenu, for chat: Chat?, store: AppStore) {
        menu.removeAllItems()
        let current = chat.flatMap { chat in chat.sectionID.flatMap(store.section)?.id }
        for section in store.sections {
            let item = NSMenuItem(title: section.name, action: #selector(RootSplitViewController.moveChatToSection(_:)), keyEquivalent: "")
            item.representedObject = section.id
            item.state = chat != nil && current == section.id ? .on : .off
            menu.addItem(item)
        }
        if !store.sections.isEmpty {
            let others = NSMenuItem(title: L("Chats", context: "no section"), action: #selector(RootSplitViewController.moveChatToSection(_:)), keyEquivalent: "")
            others.state = chat != nil && current == nil ? .on : .off
            menu.addItem(others)
            menu.addItem(.separator())
        }
        menu.addItem(NSMenuItem(title: L("New Section…"), action: #selector(RootSplitViewController.moveChatToNewSection(_:)), keyEquivalent: ""))
    }
}

/// Fills the Chat menu's Mute and Move to Section submenus for the chat on screen as they open.
@MainActor
final class ChatSubmenus: NSObject, NSMenuDelegate {
    static let shared = ChatSubmenus()

    let mute = NSMenu(title: L("Mute Chat"))
    let sections = NSMenu(title: L("Move to Section"))

    override init() {
        super.init()
        mute.delegate = self
        sections.delegate = self
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        let chat = Self.chatOnScreen
        if menu === mute {
            ChatMenus.fillMuteMenu(menu)
        } else if menu === sections {
            ChatMenus.fillSectionMenu(menu, for: chat, store: AppStore.shared)
        } else {
            // The Chat menu: Mute Chat or Unmute Chat, whichever applies.
            let muted = chat?.mute != nil
            for item in menu.items {
                if item.submenu === mute { item.isHidden = muted }
                if item.action == #selector(RootSplitViewController.unmuteChat(_:)) { item.isHidden = !muted }
            }
        }
    }

    private static var chatOnScreen: Chat? {
        guard let root = NSApp.mainWindow?.contentViewController as? RootSplitViewController,
            case let .chat(id) = root.selection
        else { return nil }
        return AppStore.shared.chat(id)
    }
}

/// The small sheets that name a section, and the one that confirms deleting it.
@MainActor
enum SectionPrompt {
    /// Asks for a section's name; `done` gets it trimmed, and is not called on Cancel or for a
    /// blank name.
    static func name(
        title: String, button: String, initial: String = "", in window: NSWindow,
        done: @escaping (String) -> Void
    ) {
        let alert = NSAlert()
        alert.messageText = title
        alert.addButton(withTitle: button)
        alert.addButton(withTitle: L("Cancel"))
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        field.stringValue = initial
        field.placeholderString = L("Section name")
        alert.accessoryView = field
        alert.beginSheetModal(for: window) { response in
            let name = AppStore.sectionName(field.stringValue)
            guard response == .alertFirstButtonReturn, !name.isEmpty else { return }
            done(name)
        }
        // The accessory view only becomes first responder once the sheet exists.
        DispatchQueue.main.async { alert.window.makeFirstResponder(field) }
    }

    /// Deleting a section keeps its chats; they move to Chats.
    static func confirmDelete(_ section: SidebarSection, in window: NSWindow, done: @escaping () -> Void) {
        let alert = NSAlert()
        alert.messageText = L("Delete “%@”?", section.name)
        alert.informativeText = L("Its chats move to %@.", L("Chats", context: "no section"))
        alert.alertStyle = .warning
        alert.addButton(withTitle: L("Delete"))
        alert.addButton(withTitle: L("Cancel"))
        alert.buttons.first?.hasDestructiveAction = true
        alert.beginSheetModal(for: window) { response in
            if response == .alertFirstButtonReturn { done() }
        }
    }
}

extension NSPasteboard.PasteboardType {
    /// A chat dragged in the sidebar, by id.
    static let lorcaChat = NSPasteboard.PasteboardType("app.lorca.sidebar.chat")
    /// A section dragged in the sidebar, by id.
    static let lorcaSection = NSPasteboard.PasteboardType("app.lorca.sidebar.section")
}
