import AppKit
import UserNotifications

/// System notifications for replies, failed responses, and pending confirmations,
/// unless read on a paired Device; a click brings the app forward on the chat. The
/// same "looking at" fact goes to the CLI (`ui.watching`), so a Runner does not push a reply
/// the user is watching arrive to their phone.
@MainActor
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    static let shared = Notifier()

    /// The chat in the main window, when the window is on screen.
    var visibleChat: () -> Chat.ID? = { nil }
    /// Opens a chat from a clicked notification.
    var openChat: (Chat.ID) -> Void = { _ in }

    private let store = AppStore.shared
    private var didAsk = false
    private var started = false
    private var pendingPermissions: Set<Message.ID> = []

    /// `UNUserNotificationCenter` needs a bundle; a bare binary (`swift run`) has none.
    private var center: UNUserNotificationCenter? {
        Bundle.main.bundleIdentifier == nil ? nil : .current()
    }

    func start() {
        guard !started else { return }
        started = true
        center?.delegate = self
        rememberPermissions()
        store.observe(self) { [weak self] event in
            guard let self else { return }
            switch event {
            case .connectionChanged: self.watchingChanged()
            case .snapshotReplaced, .identityChanged:
                self.rememberPermissions()
                self.watchingChanged()
            case let .messageAdded(chatID, messageID), let .messageChanged(chatID, messageID):
                self.permissionChanged(chatID, messageID)
            case let .messageRemoved(_, messageID): self.clearPermission(messageID)
            case let .turnFinished(chatID, botID, startedAt): self.turnFinished(chatID, botID, startedAt)
            default: break
            }
        }
        for name in [NSApplication.didBecomeActiveNotification, NSApplication.didResignActiveNotification] {
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.watchingChanged() }
            }
        }
        watchingChanged()
    }

    /// The chat the user is looking at: the app is frontmost and its window shows the chat.
    private var watchedChat: Chat.ID? {
        NSApp.isActive ? visibleChat() : nil
    }

    /// Call when the selection or the window's visibility changes.
    func watchingChanged() {
        guard started else { return }
        let watched = watchedChat
        store.setWatchedChat(watched)
        // Whatever was posted for the chat now on screen has been seen.
        if let watched { clear(watched) }
    }

    // MARK: - Posting

    private func rememberPermissions() {
        let current = Set(store.chats.flatMap(\.messages).compactMap { message -> Message.ID? in
            switch message.body {
            case let .permission(request) where request.isPending: message.id
            case let .tool(tool) where tool.run?.state == .asking: message.id
            default: nil
            }
        })
        for id in pendingPermissions.subtracting(current) { clearPermission(id) }
        pendingPermissions = current
    }

    private func clearPermission(_ messageID: Message.ID) {
        pendingPermissions.remove(messageID)
        center?.removeDeliveredNotifications(withIdentifiers: [messageID])
        center?.removePendingNotificationRequests(withIdentifiers: [messageID])
    }

    private func permissionChanged(_ chatID: Chat.ID, _ messageID: Message.ID) {
        guard let message = store.chat(chatID)?.messages.first(where: { $0.id == messageID }) else { return }
        // A permission card, or a command's card, asks.
        let asks: Bool
        switch message.body {
        case let .permission(request): asks = request.isPending
        case let .tool(tool) where tool.run != nil: asks = tool.run?.state == .asking
        default: return
        }
        guard asks else { clearPermission(messageID); return }
        guard pendingPermissions.insert(messageID).inserted else { return }
        let identityID = store.identityID
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 3_000_000_000)
            guard let self, self.store.identityID == identityID,
                let message = self.store.chat(chatID)?.messages.first(where: { $0.id == messageID }),
                let notification = ChatNotification(message), notification.kind == .permission
            else { return }
            self.post(notification, in: chatID)
        }
    }

    private func turnFinished(_ chatID: Chat.ID, _ botID: Bot.ID, _ startedAt: Date) {
        // Give the final reply and read marks from paired Devices three seconds to arrive.
        let identityID = store.identityID
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 3_000_000_000)
            guard let self, self.store.identityID == identityID else { return }
            guard let chat = self.store.chat(chatID),
                let notification = ChatNotification.finishedTurn(in: chat, botID: botID, startedAt: startedAt)
            else { return }
            self.post(notification, in: chatID)
        }
    }

    private func post(_ notification: ChatNotification, in chatID: Chat.ID) {
        guard let center, let chat = store.chat(chatID), notification.canDeliver(in: chat, watchedChat: watchedChat) else { return }

        let content = UNMutableNotificationContent()
        content.title = store.bot(notification.botID)?.name ?? store.title(for: chat)
        if !chat.isDM { content.subtitle = store.title(for: chat) }
        content.body = String(notification.body.split(whereSeparator: \.isNewline).joined(separator: " ").prefix(280))
        content.sound = .default
        content.threadIdentifier = chatID
        content.userInfo = ["chat_id": chatID]
        let request = UNNotificationRequest(identifier: notification.messageID, content: content, trigger: nil)

        let identityID = store.identityID
        Task {
            guard await self.authorized(center) else { return }
            // Notification authorization can stay open while the chat is read or answered.
            guard self.store.identityID == identityID,
                let chat = self.store.chat(chatID), notification.canDeliver(in: chat, watchedChat: self.watchedChat)
            else { return }
            do { try await center.add(request) } catch { NSLog("Posting a notification failed: \(error.localizedDescription)") }
        }
    }

    /// Asks once, the first time there is something to show.
    private func authorized(_ center: UNUserNotificationCenter) async -> Bool {
        let settings = await center.notificationSettings()
        switch settings.authorizationStatus {
        case .authorized, .provisional: return true
        case .notDetermined:
            guard !didAsk else { return false }
            didAsk = true
            return (try? await center.requestAuthorization(options: [.alert, .sound, .badge])) ?? false
        default: return false
        }
    }

    private func clear(_ chatID: Chat.ID) {
        guard let center else { return }
        Task {
            let delivered = await center.deliveredNotifications()
            let ids = delivered.filter { $0.request.content.threadIdentifier == chatID }.map(\.request.identifier)
            if !ids.isEmpty { center.removeDeliveredNotifications(withIdentifiers: ids) }
        }
    }

    // MARK: - UNUserNotificationCenterDelegate

    /// Another chat can alert while the app is frontmost; recheck in case focus just changed.
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        let chatID = notification.request.content.userInfo["chat_id"] as? String
        return await MainActor.run {
            chatID != nil && self.watchedChat == chatID ? [] : [.banner, .sound, .list]
        }
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let chatID = response.notification.request.content.userInfo["chat_id"] as? String else { return }
        await MainActor.run { Notifier.shared.openChat(chatID) }
    }
}
