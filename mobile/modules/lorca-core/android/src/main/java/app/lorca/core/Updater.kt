package app.lorca.core

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.IntentCompat
import androidx.core.content.pm.PackageInfoCompat
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/// Installs a release of the app over itself: downloads the release's APK, checks that it is the
/// file the release lists and a newer build of this app, and hands it to the system's package
/// installer, which takes it only when it is signed with the installed app's key. From Android 12
/// an app that may install packages and updates itself is not asked to confirm; otherwise, and
/// the first time, when the user has yet to let Lorca install apps, the installer asks. Either
/// way the system stops the app and replaces it.
object Updater {
  /// Where an install stands after it was handed over: the module's "update" event.
  var onStatus: ((Map<String, Any?>) -> Unit)? = null

  /// The installer's confirmation, shown once the app is in front.
  private var confirmation: Intent? = null

  /// Downloads `url` into the cache, reporting the bytes received, and checks its size and SHA-256.
  fun download(context: Context, url: String, sha256: String, size: Long, progress: (Long) -> Unit): File {
    val dir = File(context.cacheDir, "updates").apply { deleteRecursively(); mkdirs() }
    val apk = File(dir, "lorca.apk")
    val digest = MessageDigest.getInstance("SHA-256")
    // GitHub answers a release download with a redirect to its storage, which this follows.
    val connection = URL(url).openConnection() as HttpURLConnection
    connection.connectTimeout = 30_000
    connection.readTimeout = 60_000
    connection.setRequestProperty("User-Agent", "Lorca/${context.packageName}")
    try {
      if (connection.responseCode != HttpURLConnection.HTTP_OK) {
        throw IllegalStateException("The download answered ${connection.responseCode}")
      }
      var received = 0L
      var reported = -1L
      connection.inputStream.use { input ->
        apk.outputStream().use { output ->
          val buffer = ByteArray(64 * 1024)
          while (true) {
            val read = input.read(buffer)
            if (read < 0) break
            output.write(buffer, 0, read)
            digest.update(buffer, 0, read)
            received += read
            // A report per percent is plenty for a progress row.
            val step = if (size > 0) received * 100 / size else received / (1024 * 1024)
            if (step != reported) {
              reported = step
              progress(received)
            }
          }
        }
      }
      if (received != size) throw IllegalStateException("The download was $received bytes, not $size")
    } finally {
      connection.disconnect()
    }
    val hex = digest.digest().joinToString("") { "%02x".format(it) }
    if (!hex.equals(sha256, ignoreCase = true)) {
      apk.delete()
      throw IllegalStateException("The download does not match the release's SHA-256")
    }
    return apk
  }

  /// Hands `apk` to the package installer once it is this app and newer than the running build.
  fun install(context: Context, apk: File) {
    val manager = context.packageManager
    val archive = manager.getPackageArchiveInfo(apk.path, 0)
      ?: throw IllegalStateException("The download is not an Android app")
    if (archive.packageName != context.packageName) {
      throw IllegalStateException("The download is ${archive.packageName}, not ${context.packageName}")
    }
    val installed = manager.getPackageInfo(context.packageName, 0)
    if (PackageInfoCompat.getLongVersionCode(archive) <= PackageInfoCompat.getLongVersionCode(installed)) {
      throw IllegalStateException("The download is not newer than ${installed.versionName}")
    }

    val installer = manager.packageInstaller
    val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
      setAppPackageName(context.packageName)
      setSize(apk.length())
      setInstallReason(PackageManager.INSTALL_REASON_USER)
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
      }
    }
    val id = installer.createSession(params)
    try {
      installer.openSession(id).use { session ->
        session.openWrite("lorca.apk", 0, apk.length()).use { output ->
          apk.inputStream().use { it.copyTo(output) }
          session.fsync(output)
        }
        // The system fills in the status, so the intent stays mutable; it names its receiver.
        val status = Intent(context, UpdateReceiver::class.java).setPackage(context.packageName)
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or
          (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0)
        session.commit(PendingIntent.getBroadcast(context, id, status, flags).intentSender)
      }
    } catch (error: Exception) {
      installer.abandonSession(id)
      throw error
    } finally {
      // The session holds its own copy.
      apk.delete()
    }
  }

  fun received(context: Context, intent: Intent) {
    when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
      PackageInstaller.STATUS_PENDING_USER_ACTION -> {
        confirmation = IntentCompat.getParcelableExtra(intent, Intent.EXTRA_INTENT, Intent::class.java)
        onStatus?.invoke(mapOf("state" to "confirming"))
        // An app in the background may not start an activity; the module shows it on return.
        if (PushService.inFront) showConfirmation(context)
      }
      // The process that hears this is the new version's.
      PackageInstaller.STATUS_SUCCESS -> Unit
      PackageInstaller.STATUS_FAILURE_ABORTED -> onStatus?.invoke(mapOf("state" to "cancelled"))
      else -> onStatus?.invoke(
        mapOf("state" to "failed", "message" to (intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE) ?: "The install failed")),
      )
    }
  }

  /// Shows the installer's confirmation, if one waits.
  fun showConfirmation(context: Context) {
    val intent = confirmation ?: return
    confirmation = null
    if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    context.startActivity(intent)
  }
}

/// Hears how the package installer's session ended.
class UpdateReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) = Updater.received(context, intent)
}
