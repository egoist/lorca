import ExpoModulesCore

/// A container whose contents lay out for a compact width, as a split view's sidebar column
/// does. A navigation stack inside keeps its phone arrangement on an iPad: the search field at
/// the bottom rather than in the bar.
public class CompactWidthViewModule: Module {
  public func definition() -> ModuleDefinition {
    Name("CompactWidthView")
    View(CompactWidthView.self) {}
  }
}

final class CompactWidthView: ExpoView {
  required init(appContext: AppContext? = nil) {
    super.init(appContext: appContext)
    if #available(iOS 17.0, *) {
      traitOverrides.horizontalSizeClass = .compact
    }
  }
}
