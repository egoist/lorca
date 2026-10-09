package app.lorca.core

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

/// The message body view: Markdown in, a selectable native text view out. The core parses;
/// the view renders and sizes itself to its text within `maxWidth`.
class MarkdownViewModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("MarkdownView")

    // The size a message takes within `maxWidth`. Synchronous, so the size lands in the same
    // commit as the text.
    Function("measure") { markdown: String, maxWidth: Double, fontSize: Double, codeFontSize: Double ->
      val context = appContext.currentActivity ?: appContext.reactContext ?: throw IllegalStateException("No context to measure in")
      val style = MarkdownStyle().apply {
        this.markdown = markdown
        this.maxWidth = maxWidth.toFloat()
        this.fontSize = fontSize.toFloat()
        this.codeFontSize = codeFontSize.toFloat()
      }
      val (width, height) = measureMessage(context, style)
      mapOf("width" to width, "height" to height)
    }

    View(MarkdownView::class) {
      Prop("markdown") { view: MarkdownView, markdown: String -> view.style.markdown = markdown }
      Prop("maxWidth") { view: MarkdownView, width: Double -> view.style.maxWidth = width.toFloat() }
      Prop("fontSize") { view: MarkdownView, size: Double -> view.style.fontSize = size.toFloat() }
      Prop("codeFontSize") { view: MarkdownView, size: Double -> view.style.codeFontSize = size.toFloat() }
      Prop("color") { view: MarkdownView, color: Int -> view.style.color = color }
      Prop("linkColor") { view: MarkdownView, color: Int -> view.style.linkColor = color }
      Prop("codeBackground") { view: MarkdownView, color: Int -> view.style.codeBackground = color }
      Prop("quoteColor") { view: MarkdownView, color: Int -> view.style.quoteColor = color }
      Prop("border") { view: MarkdownView, color: Int -> view.style.border = color }
      Prop("tint") { view: MarkdownView, color: Int -> view.style.tint = color }
      OnViewDidUpdateProps { view: MarkdownView -> view.render() }
    }
  }
}
