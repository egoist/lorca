import ExpoModulesCore
import Security

/// The Rust core behind the phone: started once with the app's folder and the phone's facts,
/// then one request at a time and a stream of events, the same JSON the desktop app speaks
/// to the CLI over the local websocket.
public class LorcaCoreModule: Module {
  private var core: Core?

  /// Events cross from the core's threads; sendEvent hops to the JS thread itself.
  private final class Listener: EventListener, @unchecked Sendable {
    weak var module: LorcaCoreModule?
    func onEvent(json: String) {
      // Pairing and forgetting change the account key, and with it the push key.
      if json.hasPrefix("{\"event\":\"identity.changed\"") { module?.sharePushKey() }
      module?.sendEvent("event", ["json": json])
    }
  }

  /// Leaves the push key in the app group's keychain for the notification service extension,
  /// which runs outside the app and opens a push's ciphertext before the alert shows
  /// (`targets/notify`). Removed when this phone holds no account.
  fileprivate func sharePushKey() {
    let group = "group.app.lorca"
    let item: [CFString: Any] = [
      kSecClass: kSecClassGenericPassword,
      kSecAttrService: group,
      kSecAttrAccount: "push-key",
      kSecAttrAccessGroup: group,
    ]
    SecItemDelete(item as CFDictionary)
    guard let key = core?.pushKey() else { return }
    // A push can arrive while the phone is locked.
    let status = SecItemAdd(item.merging([kSecValueData: key, kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]) { $1 } as CFDictionary, nil)
    if status != errSecSuccess { NSLog("Lorca: sharing the push key failed (%d)", status) }
  }

  public func definition() -> ModuleDefinition {
    Name("LorcaCore")

    Events("event")

    Function("start") { (home: String, name: String, os: String, osVersion: String, model: String) throws in
      guard self.core == nil else { return }
      let listener = Listener()
      listener.module = self
      self.core = try Core.start(home: home, name: name, os: os, osVersion: osVersion, model: model, listener: listener)
      self.sharePushKey()
    }

    // Blocks until the core answers. On a concurrent queue, never the JS thread and never the
    // module's serial one: a request that waits (pair.accept, up to ten minutes) must not hold
    // every other request behind it.
    AsyncFunction("request") { (method: String, params: String) throws -> String in
      guard let core = self.core else { throw NotStartedException() }
      return core.request(method: method, params: params)
    }
    .runOnQueue(DispatchQueue.global(qos: .userInitiated))

    Function("wake") {
      self.core?.wake()
    }
  }
}

final class NotStartedException: Exception {
  override var reason: String { "The Lorca core has not been started" }
}
