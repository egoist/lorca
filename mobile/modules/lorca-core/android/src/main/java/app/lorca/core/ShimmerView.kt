package app.lorca.core

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.LinearGradient
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.PorterDuff
import android.graphics.PorterDuffXfermode
import android.graphics.Shader
import android.os.SystemClock
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.views.ExpoView

/// A container whose contents shimmer, as the Mac's working row does: a gradient mask dims
/// them to 40% and sweeps a bright band, 30% of the width, from the leading edge to the
/// trailing one in 1.5 s, resting a quarter second between sweeps.
class ShimmerViewModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("ShimmerView")
    View(ShimmerView::class) {}
  }
}

/// The children draw into a layer that the band then masks. Every sweep keeps time with the
/// uptime clock, so a row made again mid-sweep carries on where the last one was.
class ShimmerView(context: Context, appContext: AppContext) : ExpoView(context, appContext) {
  private val mask = Paint(Paint.ANTI_ALIAS_FLAG).apply { xfermode = PorterDuffXfermode(PorterDuff.Mode.DST_IN) }
  private val shift = Matrix()
  private var gradientWidth = 0

  override fun dispatchDraw(canvas: Canvas) {
    val w = width.toFloat()
    val h = height.toFloat()
    if (w <= 0f || h <= 0f) {
      super.dispatchDraw(canvas)
      return
    }
    if (gradientWidth != width) {
      mask.shader = LinearGradient(0f, 0f, BAND * w, 0f, intArrayOf(DIM, Color.BLACK, DIM), null, Shader.TileMode.CLAMP)
      gradientWidth = width
    }
    // The band's leading edge runs from 30% of the width before the start to the end over the
    // sweep, then waits off the start for the rest.
    val phase = (SystemClock.uptimeMillis() % PERIOD_MS).toFloat() / SWEEP_MS
    val start = if (phase < 1f) -BAND + phase * (1f + BAND) else -BAND
    shift.setTranslate(start * w, 0f)
    mask.shader.setLocalMatrix(shift)

    val layer = canvas.saveLayer(0f, 0f, w, h, null)
    super.dispatchDraw(canvas)
    canvas.drawRect(0f, 0f, w, h, mask)
    canvas.restoreToCount(layer)
    postInvalidateOnAnimation()
  }

  private companion object {
    const val BAND = 0.3f
    const val SWEEP_MS = 1500L
    const val PERIOD_MS = 1750L
    val DIM = Color.argb(102, 0, 0, 0)
  }
}
