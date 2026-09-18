import AppKit
import UserNotifications

/// System notifications for finished replies. A turn that ends with something said posts one,
/// unless the user is looking at that chat; a click brings the app forward on the chat. The
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

    /// `UNUserNotificationCenter` needs a bundle; a bare binary (`swift run`) has none.
    private var center: UNUserNotificationCenter? {
        Bundle.main.bundleIdentifier == nil ? nil : .current()
    }

    func start() {
        center?.delegate = self
        store.observe(self) { [weak self] event in
            switch event {
            case .connectionChanged, .snapshotReplaced: self?.watchingChanged()
            case let .turnFinished(chatID, botID, startedAt): self?.turnFinished(chatID, botID, startedAt)
            default: break
            }
        }
        for name in [NSApplication.didBecomeActiveNotification, NSApplication.didResignActiveNotification] {
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.watchingChanged() }
            }
        }
    }

    /// The chat the user is looking at: the app is frontmost and its window shows the chat.
    private var watchedChat: Chat.ID? {
        NSApp.isActive ? visibleChat() : nil
    }

    /// Call when the selection or the window's visibility changes.
    func watchingChanged() {
        let watched = watchedChat
        store.setWatchedChat(watched)
        // Whatever was posted for the chat now on screen has been seen.
        if let watched { clear(watched) }
    }

    // MARK: - Posting

    private func turnFinished(_ chatID: Chat.ID, _ botID: Bot.ID, _ startedAt: Date) {
        // A turn on another Runner ends with a job result that can land just ahead of the
        // last chunk of the reply; give the message a moment.
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 400_000_000)
            self?.post(chatID, botID, startedAt)
        }
    }

    private func post(_ chatID: Chat.ID, _ botID: Bot.ID, _ startedAt: Date) {
        guard watchedChat != chatID, let center, let chat = store.chat(chatID) else { return }
        // What the bot said last in this turn. A pass or a failed turn said nothing.
        let said = chat.messages.last { message in
            guard message.author.botID == botID, message.state == .complete, message.createdAt >= startedAt.addingTimeInterval(-5),
                case let .text(text) = message.body
            else { return false }
            return !text.isEmpty
        }
        guard let said, case let .text(text) = said.body else { return }

        let content = UNMutableNotificationContent()
        content.title = store.bot(botID)?.name ?? store.title(for: chat)
        if !chat.isDM { content.subtitle = store.title(for: chat) }
        content.body = String(text.split(whereSeparator: \.isNewline).joined(separator: " ").prefix(280))
        content.sound = .default
        content.threadIdentifier = chatID
        content.userInfo = ["chat_id": chatID]
        let request = UNNotificationRequest(identifier: said.id, content: content, trigger: nil)

        Task {
            guard await self.authorized(center) else { return }
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
            return (try? await center.requestAuthorization(options: [.alert, .sound])) ?? false
        default: return false
        }
    }

    private func clear(_ chatID: Chat.ID) {
        guard let center else { return }
        center.getDeliveredNotifications { delivered in
            let ids = delivered.filter { $0.request.content.threadIdentifier == chatID }.map(\.request.identifier)
            if !ids.isEmpty { center.removeDeliveredNotifications(withIdentifiers: ids) }
        }
    }

    // MARK: - UNUserNotificationCenterDelegate

    /// A reply in a chat that is not on screen shows even while the app is frontmost.
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .sound, .list]
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let chatID = response.notification.request.content.userInfo["chat_id"] as? String else { return }
        await MainActor.run { Notifier.shared.openChat(chatID) }
    }
}
