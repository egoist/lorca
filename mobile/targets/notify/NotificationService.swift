import Security
import UserNotifications

/// Runs when a push arrives, before the alert shows. The relay's alert says "New reply" and
/// carries ciphertext (`c`); this swaps in who replied, where, and the first words, opened
/// with the push key the app left in the shared keychain. Whatever fails, the fixed words show.
final class NotificationService: UNNotificationServiceExtension {
  private var deliver: ((UNNotificationContent) -> Void)?
  private var content: UNMutableNotificationContent?

  override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
    deliver = contentHandler
    guard let content = request.content.mutableCopy() as? UNMutableNotificationContent else { return contentHandler(request.content) }
    self.content = content
    if let sealed = content.userInfo["c"] as? String, let key = PushKey.read(), let notice = PushEnvelope.open(sealed, key: key) {
      content.title = notice.title
      content.subtitle = notice.subtitle ?? ""
      content.body = notice.body
      content.threadIdentifier = notice.chat_id
      // expo-notifications hands `body` to JS as the notification's data.
      content.userInfo["body"] = ["chat_id": notice.chat_id]
      content.userInfo.removeValue(forKey: "c")
    }
    contentHandler(content)
  }

  override func serviceExtensionTimeWillExpire() {
    if let deliver, let content { deliver(content) }
  }
}

/// The push key in the app group's keychain, written by the app (`LorcaCoreModule`).
enum PushKey {
  static let group = Bundle.main.bundleIdentifier?.hasPrefix("app.lorca.dev.") == true
    ? "group.app.lorca.dev"
    : "group.app.lorca"
  static let account = "push-key"

  static func read() -> Data? {
    let query: [CFString: Any] = [
      kSecClass: kSecClassGenericPassword,
      kSecAttrService: group,
      kSecAttrAccount: account,
      kSecAttrAccessGroup: group,
      kSecReturnData: true,
      kSecMatchLimit: kSecMatchLimitOne,
    ]
    var item: CFTypeRef?
    guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess else { return nil }
    return item as? Data
  }
}
