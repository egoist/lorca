package app.lorca.core

import android.content.Context
import android.graphics.BitmapFactory
import android.media.ExifInterface
import android.net.Uri
import android.view.View
import android.webkit.MimeTypeMap
import android.widget.EditText
import androidx.core.view.ContentInfoCompat
import androidx.core.view.OnReceiveContentListener
import androidx.core.view.ViewCompat
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import java.io.File
import java.util.UUID
import java.util.concurrent.Executors

/// A container that takes images put into the text input inside it: pasted from the clipboard,
/// inserted by the keyboard, or dropped on it. Each image is copied to a file and the files go
/// to `onPasteImages`; text that came with them lands in the field as usual.
class ImagePasteViewModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("ImagePasteView")
    View(ImagePasteView::class) {
      Events("onPasteImages")
    }
  }
}

class ImagePasteView(context: Context, appContext: AppContext) : ExpoView(context, appContext) {
  private val onPasteImages by EventDispatcher()

  /// Called for every paste, keyboard insertion, and drop; what is not an image goes back to
  /// the field's own handling.
  private val receiver = OnReceiveContentListener { view, payload ->
    val split = payload.partition { item -> item.uri?.let { mimeOf(view.context, it, payload) } != null }
    split.first?.let { images ->
      val uris = (0 until images.clip.itemCount).mapNotNull { images.clip.getItemAt(it).uri }
      receive(view.context, uris, payload)
    }
    split.second
  }

  override fun onViewAdded(child: View) {
    super.onViewAdded(child)
    if (child is EditText) ViewCompat.setOnReceiveContentListener(child, MIME_TYPES, receiver)
  }

  override fun onViewRemoved(child: View) {
    if (child is EditText) ViewCompat.setOnReceiveContentListener(child, null, null)
    super.onViewRemoved(child)
  }

  private fun receive(context: Context, uris: List<Uri>, payload: ContentInfoCompat) {
    val mimes = uris.map { mimeOf(context, it, payload) ?: "image/png" }
    copier.execute {
      val files = uris.zip(mimes).mapNotNull { (uri, mime) -> runCatching { copy(context, uri, mime) }.getOrNull() }
      if (files.isNotEmpty()) post { onPasteImages(mapOf("files" to files)) }
    }
  }

  private fun copy(context: Context, uri: Uri, mime: String): Map<String, Any> {
    val directory = File(context.cacheDir, "Pasted").apply { mkdirs() }
    val extension = MimeTypeMap.getSingleton().getExtensionFromMimeType(mime) ?: "png"
    val file = File(directory, "${UUID.randomUUID()}.$extension")
    val input = context.contentResolver.openInputStream(uri) ?: throw IllegalStateException("Cannot open $uri")
    input.use { source -> file.outputStream().use { source.copyTo(it) } }
    val result = mutableMapOf<String, Any>("uri" to Uri.fromFile(file).toString(), "mime" to mime, "size" to file.length().toDouble())
    pixelSize(file)?.let { (width, height) ->
      result["width"] = width
      result["height"] = height
    }
    return result
  }

  /// The size as shown: an EXIF orientation of 5 through 8 turns the image on its side.
  private fun pixelSize(file: File): Pair<Int, Int>? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(file.path, bounds)
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    val orientation = runCatching { ExifInterface(file.path).getAttributeInt(ExifInterface.TAG_ORIENTATION, 1) }.getOrDefault(1)
    return if (orientation >= 5) bounds.outHeight to bounds.outWidth else bounds.outWidth to bounds.outHeight
  }

  private companion object {
    val MIME_TYPES = arrayOf("image/*")
    val copier = Executors.newSingleThreadExecutor()

    /// The item's image type, from its provider or else from the clip's description.
    fun mimeOf(context: Context, uri: Uri, payload: ContentInfoCompat): String? {
      context.contentResolver.getType(uri)?.let { return if (it.startsWith("image/")) it else null }
      val description = payload.clip.description
      return (0 until description.mimeTypeCount).map { description.getMimeType(it) }.firstOrNull { it.startsWith("image/") }
    }
  }
}
