package app.lorca.core

import expo.modules.kotlin.functions.Coroutine
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.lorca_mobile.Core
import uniffi.lorca_mobile.EventListener

/// The Rust core behind the phone: started once with the app's folder and the phone's facts,
/// then one request at a time and a stream of events.
class LorcaCoreModule : Module() {
  private var core: Core? = null

  override fun definition() = ModuleDefinition {
    Name("LorcaCore")

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

    // Blocks until the core answers. On the IO pool, never the JS thread and never the
    // module's single queue thread: a request that waits (pair.accept, up to ten minutes)
    // must not hold every other request behind it.
    AsyncFunction("request").Coroutine { method: String, params: String ->
      val running = core ?: throw IllegalStateException("The Lorca core has not been started")
      withContext(Dispatchers.IO) { running.request(method, params) }
    }

    Function("wake") {
      core?.wake()
    }

    // The chat on screen, or null. While the app is in front PushService posts nothing for
    // it, and whatever it posted for it has now been seen.
    Function("setOpenChat") { chatId: String? ->
      PushService.openChat = chatId
      if (chatId != null && PushService.inFront) clearPosted(chatId)
    }

    OnActivityEntersForeground {
      PushService.inFront = true
      PushService.openChat?.let { clearPosted(it) }
    }

    OnActivityEntersBackground {
      PushService.inFront = false
    }
  }

  private fun clearPosted(chatId: String) {
    appContext.reactContext?.let { PushService.clear(it, chatId) }
  }
}
