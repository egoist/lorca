import AppKit
import XCTest

@testable import Lorca

@MainActor
final class SidebarSectionTests: XCTestCase {
    private func chat(_ id: String, section: String? = nil, pinned: Bool = false, hidden: Bool = false) -> Chat {
        Chat(
            id: id, kind: .dm, customTitle: nil, botIDs: ["bot"], messages: [], unreadCount: 0, isPinned: pinned,
            createdAt: Date(timeIntervalSince1970: 0), sectionID: section, isHidden: hidden)
    }

    func testWithoutSectionsTheChatsAreOneListAndHiddenOnesFoldAway() {
        let chats = [chat("pinned", pinned: true), chat("a"), chat("gone", hidden: true)]
        let layout = SidebarLayout(chats: chats, sections: [], showsHidden: false, collapsesOthers: false)
        XCTAssertEqual(layout.entries, [
            .chat("pinned"), .chat("a"),
            .group(.init(kind: .hidden, chatIDs: ["gone"], isCollapsed: true)),
        ])
        XCTAssertEqual(SidebarLayout(chats: [chat("a")], sections: [], showsHidden: false, collapsesOthers: false).entries, [.chat("a")], "nothing hidden, no Hidden")
    }

    func testSectionsListTheirChatsAndTheRestFollowUnderChats() {
        let sections = [SidebarSection(id: "s1", name: "Pipeline", isCollapsed: true), SidebarSection(id: "s2", name: "Empty")]
        let chats = [
            chat("pinned", section: "s1", pinned: true), chat("deal", section: "s1"), chat("loose"),
            chat("orphan", section: "deleted"), chat("hidden", section: "s1", hidden: true),
        ]
        let layout = SidebarLayout(chats: chats, sections: sections, showsHidden: true, collapsesOthers: false)
        XCTAssertEqual(layout.entries, [
            .chat("pinned"),
            .group(.init(kind: .section("s1"), chatIDs: ["deal"], isCollapsed: true)),
            .group(.init(kind: .section("s2"), chatIDs: [], isCollapsed: false)),
            .group(.init(kind: .others, chatIDs: ["loose", "orphan"], isCollapsed: false)),
            .group(.init(kind: .hidden, chatIDs: ["hidden"], isCollapsed: false)),
        ])
        let filed = SidebarLayout(chats: [chat("deal", section: "s1")], sections: sections, showsHidden: false, collapsesOthers: false)
        XCTAssertFalse(filed.groups.contains { $0.kind == .others }, "an empty Chats group shows nothing")
    }

    func testMutedUntilATimeEndsOnItsOwn() {
        var muted = chat("a")
        muted.mute = Chat.Mute(until: Date(timeIntervalSince1970: 100))
        XCTAssertTrue(muted.isMuted(at: Date(timeIntervalSince1970: 99)))
        XCTAssertFalse(muted.isMuted(at: Date(timeIntervalSince1970: 100)))
        muted.mute = Chat.Mute(until: nil)
        XCTAssertTrue(muted.isMuted(at: .distantFuture))
    }

    /// The real sidebar over the demo roster: sections with headers that fold, the chats
    /// moving between them as the store says, and the Hidden group.
    func testTheSidebarFollowsTheStore() async throws {
        guard AppStore.shared.isMock else { throw XCTSkip("The sidebar runs against the demo roster: set LORCA_MOCK=1") }
        _ = NSApplication.shared
        let store = AppStore.shared
        store.start()
        store.resetMockData()
        Preferences.showsHiddenChats = false
        Preferences.collapsesOtherChats = false
        defer { store.resetMockData() }

        let sidebar = SidebarViewController()
        let window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 300, height: 860), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = sidebar
        window.setContentSize(NSSize(width: 300, height: 860))
        window.orderFront(nil)
        defer { window.close() }
        sidebar.view.layoutSubtreeIfNeeded()
        let outline = try XCTUnwrap(descendants(sidebar.view).compactMap { $0 as? NSOutlineView }.first)

        func rows() -> [String] {
            (0..<outline.numberOfRows).compactMap { row in
                guard let node = outline.item(atRow: row) as? SidebarNode else { return nil }
                switch node.kind {
                case let .chat(id): return id
                case let .group(group): return "[\((outline.view(atColumn: 0, row: row, makeIfNecessary: true) as? SidebarHeaderCell).map(headerTitle) ?? "\(group)")]"
                default: return nil
                }
            }
        }
        /// The rows by group, "" for those above the first header; within a group, the store's order.
        func groups() -> [(String, Set<String>)] {
            rows().reduce(into: [("", Set<String>())]) { groups, row in
                if row.hasPrefix("[") { groups.append((row, [])) } else { groups[groups.count - 1].1.insert(row) }
            }
        }
        func members(_ header: String) -> Set<String> { groups().first { $0.0 == header }?.1 ?? [] }
        let loose = Set(store.chats.filter { !$0.isPinned && $0.sectionID == nil && !$0.isHidden }.map(\.id))
        XCTAssertEqual(groups().map(\.0), ["", "[Product]", "[Engineering]", "[Chats]"])
        XCTAssertEqual(members(""), ["chat-relay"])
        XCTAssertEqual(members("[Product]"), ["chat-nova", "chat-launch"])
        XCTAssertEqual(members("[Engineering]"), ["chat-patch", "chat-ember"])
        XCTAssertEqual(members("[Chats]"), loose)
        XCTAssertTrue(loose.isSuperset(of: ["chat-quill", "chat-scout"]))
        try capture(outline, window: window, name: "mac-sidebar-sections")

        // A header is a group row, which folds through the Show/Hide control AppKit shows on hover.
        XCTAssertTrue(outline.delegate?.outlineView?(outline, isGroupItem: outline.item(atRow: 1) as Any) ?? false)
        XCTAssertTrue(outline.isExpandable(outline.item(atRow: 1)))
        XCTAssertEqual(outline.frameOfCell(atColumn: 0, row: 2).minX, outline.frameOfCell(atColumn: 0, row: 0).minX, "rows in a section line up with the pinned rows")

        // Folding a section is the store's, and so every Device's.
        let product = try XCTUnwrap(outline.item(atRow: 1) as? SidebarNode)
        outline.collapseItem(product)
        XCTAssertEqual(store.section("section-product")?.isCollapsed, true)
        XCTAssertEqual(rows().prefix(3), ["chat-relay", "[Product]", "[Engineering]"])
        store.setSectionCollapsed("section-product", false)
        XCTAssertEqual(members("[Product]"), ["chat-nova", "chat-launch"])

        // A chat moved to a section comes off the pinned rows; one hidden goes to a folded Hidden.
        store.moveChat("chat-relay", toSection: "section-engineering")
        store.setHidden("chat-quill", true)
        store.mute("chat-scout", until: Date().addingTimeInterval(3600))
        XCTAssertEqual(groups().map(\.0), ["", "[Product]", "[Engineering]", "[Chats]", "[Hidden]"])
        XCTAssertEqual(members(""), [])
        XCTAssertEqual(members("[Engineering]"), ["chat-relay", "chat-patch", "chat-ember"])
        XCTAssertEqual(members("[Chats]"), loose.subtracting(["chat-quill"]))
        XCTAssertEqual(members("[Hidden]"), [], "Hidden starts folded")
        XCTAssertEqual(store.chat("chat-relay")?.isPinned, false)

        // Opening a hidden chat some other way unfolds Hidden to show its row, selected.
        sidebar.setSelection(.chat("chat-quill"))
        XCTAssertEqual(rows().suffix(2), ["[Hidden]", "chat-quill"])
        XCTAssertEqual((outline.item(atRow: outline.selectedRow) as? SidebarNode)?.chatID, "chat-quill")
        XCTAssertTrue(Preferences.showsHiddenChats)
        try capture(outline, window: window, name: "mac-sidebar-hidden")

        // Folded again around the chat on screen, Hidden stays folded until the chat is opened.
        let hidden = try XCTUnwrap(outline.item(atRow: rows().count - 2) as? SidebarNode)
        var picked: [Selection?] = []
        sidebar.onSelect = { picked.append($0) }
        outline.collapseItem(hidden)
        XCTAssertEqual(rows().last, "[Hidden]")
        XCTAssertTrue(picked.isEmpty, "folding the open chat's row away leaves the chat open")
        sidebar.setSelection(.chat("chat-quill"))
        XCTAssertEqual(rows().last, "[Hidden]")
        sidebar.reveal("chat-quill")
        XCTAssertEqual(rows().last, "chat-quill")

        // A new section goes after the others; deleting one leaves its chats under Chats.
        store.createSection(named: "  Customers ", moving: "chat-patch")
        XCTAssertEqual(store.sections.map(\.name), ["Product", "Engineering", "Customers"])
        store.moveSection("section-engineering", to: 0)
        store.deleteSection("section-product")
        XCTAssertEqual(groups().map(\.0), ["", "[Engineering]", "[Customers]", "[Chats]", "[Hidden]"])
        XCTAssertEqual(members("[Engineering]"), ["chat-relay", "chat-ember"])
        XCTAssertEqual(members("[Customers]"), ["chat-patch"])
        XCTAssertEqual(members("[Chats]"), loose.subtracting(["chat-quill"]).union(["chat-nova", "chat-launch"]))
        XCTAssertEqual(members("[Hidden]"), ["chat-quill"])
        Preferences.showsHiddenChats = false
    }

    private func headerTitle(_ cell: SidebarHeaderCell) -> String {
        descendants(cell).compactMap { $0 as? NSTextField }.first?.stringValue ?? ""
    }

    /// The list as it shows, in each appearance, when `LORCA_UI_CAPTURE_DIR` names a folder.
    private func capture(_ view: NSView, window: NSWindow, name: String) throws {
        guard let folder = ProcessInfo.processInfo.environment["LORCA_UI_CAPTURE_DIR"] else { return }
        for (suffix, appearance) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)] {
            window.appearance = NSAppearance(named: appearance)
            view.layoutSubtreeIfNeeded()
            view.displayIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            try FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
            try png.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name)-\(suffix).png"))
        }
    }

    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
}
