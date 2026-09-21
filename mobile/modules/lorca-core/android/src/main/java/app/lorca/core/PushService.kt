package app.lorca.core

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.net.Uri
import androidx.core.app.NotificationCompat
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import java.io.File
import uniffi.lorca_mobile.pushOpen

/// A push is a data message carrying ciphertext (`c`). The core opens it with the account key
/// under the app's folder, no running core needed, and the notification shows who replied,
/// where, and the first words. A tap opens the chat through this build's app link.
class PushService : FirebaseMessagingService() {
  override fun onMessageReceived(message: RemoteMessage) {
    val sealed = message.data["c"] ?: return
    // The folder the JS side starts the core with (`coreHome()` in src/core/prefs.ts).
    val development = packageName == "app.lorca.dev"
    val home = File(filesDir, "${if (development) "lorca-dev" else "lorca"}/core").absolutePath
    val notice = try { pushOpen(home, sealed) } catch (_: Throwable) { null }

    val manager = getSystemService(NotificationManager::class.java)
    manager.createNotificationChannel(NotificationChannel(CHANNEL, "Replies", NotificationManager.IMPORTANCE_HIGH))
    val scheme = if (development) "lorca-dev" else "lorca"
    val open = Intent(Intent.ACTION_VIEW, Uri.parse(if (notice != null) "$scheme://chat/${notice.chatId}" else "$scheme://")).setPackage(packageName)
    val tap = PendingIntent.getActivity(this, notice?.chatId.hashCode(), open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    val title = when {
      notice == null -> applicationInfo.loadLabel(packageManager).toString()
      notice.subtitle != null -> "${notice.title} · ${notice.subtitle}"
      else -> notice.title
    }
    val body = notice?.body ?: "New reply"
    val notification = NotificationCompat.Builder(this, CHANNEL)
      .setSmallIcon(applicationInfo.icon)
      .setContentTitle(title)
      .setContentText(body)
      .setStyle(NotificationCompat.BigTextStyle().bigText(body))
      .setGroup(notice?.chatId)
      .setAutoCancel(true)
      .setContentIntent(tap)
      .build()
    manager.notify(notice?.chatId ?: "lorca", (message.messageId ?: body).hashCode(), notification)
  }

  // The app registers the current token with the relay each time it starts (src/core/push.ts).
  override fun onNewToken(token: String) {}

  private companion object {
    const val CHANNEL = "replies"
  }
}
