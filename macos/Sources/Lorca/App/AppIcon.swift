import AppKit

/// Loads the icon shared by the app bundle, Dock, and onboarding.
enum AppIcon {
    static func make(size: CGFloat = 512) -> NSImage {
        let iconFile = Bundle.main.object(forInfoDictionaryKey: "CFBundleIconFile") as? String
            ?? "Lorca.icns"
        let resource = iconFile as NSString
        guard let url = Bundle.main.url(
            forResource: resource.deletingPathExtension,
            withExtension: resource.pathExtension.isEmpty ? "icns" : resource.pathExtension
        ),
            let image = NSImage(contentsOf: url)
        else { return NSImage(size: NSSize(width: size, height: size)) }

        image.size = NSSize(width: size, height: size)
        return image
    }
}
