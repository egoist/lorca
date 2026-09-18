import AppKit

/// The dock icon is drawn at runtime so the repo carries no binary assets.
enum AppIcon {
    static func make(size: CGFloat = 512) -> NSImage {
        NSImage(size: NSSize(width: size, height: size), flipped: false) { rect in
            let unit = rect.width / 512

            let plate = NSBezierPath(
                roundedRect: rect.insetBy(dx: 26 * unit, dy: 26 * unit),
                xRadius: 104 * unit,
                yRadius: 104 * unit
            )
            let gradient = NSGradient(
                colors: [
                    NSColor(srgbRed: 0.42, green: 0.40, blue: 0.96, alpha: 1),
                    NSColor(srgbRed: 0.24, green: 0.52, blue: 0.94, alpha: 1),
                    NSColor(srgbRed: 0.18, green: 0.70, blue: 0.86, alpha: 1),
                ],
                atLocations: [0, 0.55, 1],
                colorSpace: .sRGB
            )
            gradient?.draw(in: plate, angle: -90)

            NSColor.white.setFill()
            NSColor.white.setStroke()

            // Antenna
            let antenna = NSBezierPath()
            antenna.move(to: NSPoint(x: rect.midX, y: 352 * unit))
            antenna.line(to: NSPoint(x: rect.midX, y: 392 * unit))
            antenna.lineWidth = 16 * unit
            antenna.lineCapStyle = .round
            antenna.stroke()
            NSBezierPath(
                ovalIn: NSRect(x: rect.midX - 21 * unit, y: 388 * unit, width: 42 * unit, height: 42 * unit)
            ).fill()

            // Head
            let head = NSRect(x: 118 * unit, y: 132 * unit, width: 276 * unit, height: 224 * unit)
            NSBezierPath(roundedRect: head, xRadius: 74 * unit, yRadius: 74 * unit).fill()

            // Eyes, punched back out in the plate color
            NSColor(srgbRed: 0.27, green: 0.46, blue: 0.95, alpha: 1).setFill()
            let eyeSize = 44 * unit
            let eyeY = head.midY - eyeSize / 2 + 6 * unit
            NSBezierPath(
                ovalIn: NSRect(x: head.midX - 76 * unit, y: eyeY, width: eyeSize, height: eyeSize)
            ).fill()
            NSBezierPath(
                ovalIn: NSRect(x: head.midX + 32 * unit, y: eyeY, width: eyeSize, height: eyeSize)
            ).fill()

            return true
        }
    }
}
