import ExpoModulesCore

/// A container that gives the scroll view inside it the soft top edge a phone's has. An
/// iPad's automatic style is the hard one, which rules a line under a transparent bar.
public class SoftScrollEdgeViewModule: Module {
  public func definition() -> ModuleDefinition {
    Name("SoftScrollEdgeView")
    View(SoftScrollEdgeView.self) {}
  }
}

final class SoftScrollEdgeView: ExpoView {
  private weak var styled: UIScrollView?

  override func layoutSubviews() {
    super.layoutSubviews()
    guard #available(iOS 26.0, *), styled == nil, let scrollView = Self.scrollView(in: self) else { return }
    scrollView.topEdgeEffect.style = .soft
    styled = scrollView
  }

  private static func scrollView(in view: UIView) -> UIScrollView? {
    for child in view.subviews {
      if let found = child as? UIScrollView ?? scrollView(in: child) { return found }
    }
    return nil
  }
}
