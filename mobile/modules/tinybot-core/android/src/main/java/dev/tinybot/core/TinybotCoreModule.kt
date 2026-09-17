package dev.tinybot.core

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import uniffi.tinybot_mobile.Core
import uniffi.tinybot_mobile.EventListener

/// The Rust core behind the phone: started once with the app's folder and the phone's facts,
/// then one request at a time and a stream of events.
class TinybotCoreModule : Module() {
  private var core: Core? = null

  override fun definition() = ModuleDefinition {
    Name("TinybotCore")

    Events("event")

    Function("start") { home: String, name: String, os: String, osVersion: String, model: String ->
      if (core == null) {
        core = Core.start(home, name, os, osVersion, model, object : EventListener {
          override fun onEvent(json: String) {
            sendEvent("event", mapOf("json" to json))
          }
        })
      }
    }

    // Blocks until the core answers, on the module's background thread, never the JS thread.
    AsyncFunction("request") { method: String, params: String ->
      val running = core ?: throw IllegalStateException("The Tinybot core has not been started")
      running.request(method, params)
    }

    Function("wake") {
      core?.wake()
    }
  }
}
