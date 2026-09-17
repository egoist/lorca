import ExpoModulesCore

/// The Rust core behind the phone: started once with the app's folder and the phone's facts,
/// then one request at a time and a stream of events, the same JSON the desktop app speaks
/// to the CLI over the local websocket.
public class TinybotCoreModule: Module {
  private var core: Core?

  /// Events cross from the core's threads; sendEvent hops to the JS thread itself.
  private final class Listener: EventListener, @unchecked Sendable {
    weak var module: TinybotCoreModule?
    func onEvent(json: String) {
      module?.sendEvent("event", ["json": json])
    }
  }

  public func definition() -> ModuleDefinition {
    Name("TinybotCore")

    Events("event")

    Function("start") { (home: String, name: String, os: String, osVersion: String, model: String) throws in
      guard self.core == nil else { return }
      let listener = Listener()
      listener.module = self
      self.core = try Core.start(home: home, name: name, os: os, osVersion: osVersion, model: model, listener: listener)
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
  override var reason: String { "The Tinybot core has not been started" }
}
