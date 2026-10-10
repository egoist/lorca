import ExpoModulesCore
import ImageIO
import ObjectiveC
import UIKit
import UniformTypeIdentifiers

/// A container that takes images pasted into the text input inside it: Paste, offered in the
/// edit menu while the pasteboard holds an image, writes each image to a file and hands the
/// files to `onPasteImages`. Text on the pasteboard alongside the images still lands in the field.
public class ImagePasteViewModule: Module {
  public func definition() -> ModuleDefinition {
    Name("ImagePasteView")
    OnCreate { ImagePasteView.hookTextViews() }
    View(ImagePasteView.self) {
      Events("onPasteImages")
    }
  }
}

final class ImagePasteView: ExpoView {
  let onPasteImages = EventDispatcher()

  /// The images on the pasteboard, read here on the main thread as the paste happens, then
  /// written out off it.
  func pasteImages() {
    let pasteboard = UIPasteboard.general
    var images: [(Data, UTType)] = []
    for index in 0..<pasteboard.numberOfItems {
      let item = IndexSet(integer: index)
      let types = pasteboard.types(forItemSet: item)?.first ?? []
      guard let name = types.first(where: { UTType($0)?.conforms(to: .image) == true }),
            let type = UTType(name),
            let data = pasteboard.data(forPasteboardType: name, inItemSet: item)?.first as? Data
      else { continue }
      images.append((data, type))
    }
    // An image put on the pasteboard as a UIImage has no data of its own.
    if images.isEmpty {
      images = (pasteboard.images ?? []).compactMap { image in image.pngData().map { ($0, UTType.png) } }
    }
    guard !images.isEmpty else { return }
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let files = images.compactMap { Self.write($0.0, type: $0.1) }
      DispatchQueue.main.async {
        guard let self, !files.isEmpty else { return }
        self.onPasteImages(["files": files])
      }
    }
  }

  /// PNG, JPEG, GIF, and WebP go as they are; any other image (HEIC, TIFF) as a JPEG, which
  /// every provider takes.
  private static func write(_ data: Data, type: UTType) -> [String: Any]? {
    var data = data
    var type = type
    if ![UTType.png, .jpeg, .gif, .webP].contains(where: { type.conforms(to: $0) }) {
      guard let jpeg = UIImage(data: data)?.jpegData(compressionQuality: 0.85) else { return nil }
      data = jpeg
      type = .jpeg
    }
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("Pasted", isDirectory: true)
    let url = directory.appendingPathComponent(UUID().uuidString).appendingPathExtension(type.preferredFilenameExtension ?? "png")
    do {
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
      try data.write(to: url)
    } catch {
      return nil
    }
    var file: [String: Any] = ["uri": url.absoluteString, "mime": type.preferredMIMEType ?? "image/png", "size": data.count]
    if let (width, height) = pixelSize(data) {
      file["width"] = width
      file["height"] = height
    }
    return file
  }

  /// The size as shown: an EXIF orientation of 5 through 8 turns the image on its side.
  private static func pixelSize(_ data: Data) -> (Int, Int)? {
    guard let source = CGImageSourceCreateWithData(data as CFData, nil),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int
    else { return nil }
    let orientation = properties[kCGImagePropertyOrientation] as? Int ?? 1
    return orientation >= 5 ? (height, width) : (width, height)
  }

  // MARK: - The text view's paste

  private static var hooked = false

  /// React Native's multiline input is an `RCTUITextView`, made by its own component view, so
  /// it cannot be subclassed from here. Its `paste:` and `canPerformAction:withSender:` are
  /// replaced instead, once, and the replacements change nothing for a text view outside an
  /// `ImagePasteView`.
  static func hookTextViews() {
    guard !hooked, let textView: AnyClass = NSClassFromString("RCTUITextView") else { return }
    hooked = true

    let pasteSelector = #selector(UIResponder.paste(_:))
    replace(textView, pasteSelector) { original in
      typealias Paste = @convention(c) (AnyObject, Selector, Any?) -> Void
      let paste = unsafeBitCast(original, to: Paste.self)
      let block: @convention(block) (UIView, Any?) -> Void = { view, sender in
        let pasteboard = UIPasteboard.general
        guard pasteboard.hasImages, let host = host(of: view) else {
          paste(view, pasteSelector, sender)
          return
        }
        host.pasteImages()
        if pasteboard.hasStrings { paste(view, pasteSelector, sender) }
      }
      return unsafeBitCast(block, to: AnyObject.self)
    }

    let canPerformSelector = #selector(UIResponder.canPerformAction(_:withSender:))
    replace(textView, canPerformSelector) { original in
      typealias CanPerform = @convention(c) (AnyObject, Selector, Selector, Any?) -> Bool
      let canPerform = unsafeBitCast(original, to: CanPerform.self)
      let block: @convention(block) (UIView, Selector, Any?) -> Bool = { view, action, sender in
        if canPerform(view, canPerformSelector, action, sender) { return true }
        // UITextView offers Paste only for text; an image on the pasteboard is enough here.
        return action == pasteSelector && (view as? UITextView)?.isEditable == true && UIPasteboard.general.hasImages && host(of: view) != nil
      }
      return unsafeBitCast(block, to: AnyObject.self)
    }
  }

  /// Gives `cls` its own implementation of `selector`, made from the one it has now: its own,
  /// or the one it inherits, which is then left alone in the superclass.
  private static func replace(_ cls: AnyClass, _ selector: Selector, _ make: (IMP) -> AnyObject) {
    guard let method = class_getInstanceMethod(cls, selector) else { return }
    let implementation = imp_implementationWithBlock(make(method_getImplementation(method)))
    if !class_addMethod(cls, selector, implementation, method_getTypeEncoding(method)) {
      method_setImplementation(method, implementation)
    }
  }

  private static func host(of view: UIView) -> ImagePasteView? {
    var ancestor = view.superview
    while let current = ancestor {
      if let host = current as? ImagePasteView { return host }
      ancestor = current.superview
    }
    return nil
  }
}
