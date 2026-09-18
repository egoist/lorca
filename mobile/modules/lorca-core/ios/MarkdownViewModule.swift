import ExpoModulesCore

/// The message body view: Markdown in, a selectable native text view out. The core parses;
/// the view renders and sizes itself to its text within `maxWidth`.
public class MarkdownViewModule: Module {
  public func definition() -> ModuleDefinition {
    Name("MarkdownView")

    // The size a message takes within `maxWidth`. Synchronous, so the size lands in the same
    // commit as the text.
    Function("measure") { (markdown: String, maxWidth: Double, fontSize: Double, codeFontSize: Double) -> [String: Double] in
      var style = MarkdownStyle()
      style.markdown = markdown
      style.maxWidth = CGFloat(maxWidth)
      style.fontSize = CGFloat(fontSize)
      style.codeFontSize = CGFloat(codeFontSize)
      let size = MarkdownMeasure.message(style)
      return ["width": Double(size.width), "height": Double(size.height)]
    }

    View(MarkdownView.self) {
      Prop("markdown") { (view: MarkdownView, markdown: String) in
        view.style.markdown = markdown
      }
      Prop("maxWidth") { (view: MarkdownView, width: Double) in
        view.style.maxWidth = CGFloat(width)
      }
      Prop("fontSize") { (view: MarkdownView, size: Double) in
        view.style.fontSize = CGFloat(size)
      }
      Prop("codeFontSize") { (view: MarkdownView, size: Double) in
        view.style.codeFontSize = CGFloat(size)
      }
      Prop("color") { (view: MarkdownView, color: UIColor) in
        view.style.color = color
      }
      Prop("linkColor") { (view: MarkdownView, color: UIColor) in
        view.style.linkColor = color
      }
      Prop("codeBackground") { (view: MarkdownView, color: UIColor) in
        view.style.codeBackground = color
      }
      Prop("quoteColor") { (view: MarkdownView, color: UIColor) in
        view.style.quoteColor = color
      }
      Prop("border") { (view: MarkdownView, color: UIColor) in
        view.style.border = color
      }
      Prop("tint") { (view: MarkdownView, color: UIColor) in
        view.style.tint = color
      }
      OnViewDidUpdateProps { (view: MarkdownView) in
        view.render()
      }
    }
  }
}
