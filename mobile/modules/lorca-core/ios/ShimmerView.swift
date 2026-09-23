import ExpoModulesCore

/// A container whose contents shimmer, as the Mac's working row does: a gradient mask dims
/// them to 40% and sweeps a bright band, 30% of the width, from the leading edge to the
/// trailing one in 1.5 s, resting a quarter second between sweeps.
public class ShimmerViewModule: Module {
  public func definition() -> ModuleDefinition {
    Name("ShimmerView")
    View(ShimmerView.self) {}
  }
}

final class ShimmerView: ExpoView {
  /// The children live in a view of this one's own: React Native clears the mask of the view
  /// it styles every time it restyles it.
  private let content = UIView()
  private let shimmer = ShimmerLayer()

  required init(appContext: AppContext? = nil) {
    super.init(appContext: appContext)
    content.layer.mask = shimmer
    addSubview(content)
  }

  override func mountChildComponentView(_ childComponentView: UIView, index: Int) {
    content.insertSubview(childComponentView, at: index)
  }

  override func unmountChildComponentView(_ childComponentView: UIView, index: Int) {
    childComponentView.removeFromSuperview()
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    content.frame = bounds
    shimmer.frame = content.bounds
  }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    window == nil ? shimmer.stop() : shimmer.start()
  }
}

/// The mask. Every sweep keeps time with the media clock, so a row made again mid-sweep (a
/// list re-render) carries on where the last one was instead of starting over.
private final class ShimmerLayer: CAGradientLayer {
  private static let band: CGFloat = 0.3
  private static let sweep: CFTimeInterval = 1.5
  private static let rest: CFTimeInterval = 0.25

  override init() {
    super.init()
    let dim = UIColor.black.withAlphaComponent(0.4).cgColor
    colors = [dim, UIColor.black.cgColor, dim]
    startPoint = CGPoint(x: -Self.band, y: 0.5)
    endPoint = CGPoint(x: 0, y: 0.5)
  }

  override init(layer: Any) {
    super.init(layer: layer)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError() }

  /// The mask follows its view's frame at once.
  override func action(forKey event: String) -> CAAction? { nil }

  func start() {
    guard animation(forKey: "shimmer") == nil else { return }
    let start = CABasicAnimation(keyPath: "startPoint")
    start.fromValue = NSValue(cgPoint: CGPoint(x: -Self.band, y: 0.5))
    start.toValue = NSValue(cgPoint: CGPoint(x: 1, y: 0.5))
    let end = CABasicAnimation(keyPath: "endPoint")
    end.fromValue = NSValue(cgPoint: CGPoint(x: 0, y: 0.5))
    end.toValue = NSValue(cgPoint: CGPoint(x: 1 + Self.band, y: 0.5))
    start.duration = Self.sweep
    end.duration = Self.sweep

    let shimmer = CAAnimationGroup()
    shimmer.animations = [start, end]
    shimmer.duration = Self.sweep + Self.rest
    shimmer.repeatCount = .infinity
    let now = convertTime(CACurrentMediaTime(), from: nil)
    shimmer.beginTime = now - now.truncatingRemainder(dividingBy: shimmer.duration)
    add(shimmer, forKey: "shimmer")
  }

  func stop() {
    removeAnimation(forKey: "shimmer")
  }
}
