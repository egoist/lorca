import AppKit

/// Shimmering words, after SwiftUI-Shimmer's mask mode. As a view's layer mask it dims what
/// it covers to 40% and sweeps a bright band, 30% of the view's width, from the leading edge
/// to the trailing one in 1.5 s, resting a quarter second between sweeps.
///
/// Every sweep keeps time with the media clock, so a view made again mid-sweep (a table
/// reload) carries on where the last one was instead of starting over.
final class ShimmerLayer: CAGradientLayer {
    private static let band: CGFloat = 0.3
    private static let sweep: CFTimeInterval = 1.5
    private static let rest: CFTimeInterval = 0.25

    override init() {
        super.init()
        let dim = NSColor.black.withAlphaComponent(0.4).cgColor
        colors = [dim, NSColor.black.cgColor, dim]
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
        start.fromValue = NSValue(point: CGPoint(x: -Self.band, y: 0.5))
        start.toValue = NSValue(point: CGPoint(x: 1, y: 0.5))
        let end = CABasicAnimation(keyPath: "endPoint")
        end.fromValue = NSValue(point: CGPoint(x: 0, y: 0.5))
        end.toValue = NSValue(point: CGPoint(x: 1 + Self.band, y: 0.5))
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
