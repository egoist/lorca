import CryptoKit
import Foundation

/// What a Runner seals into a push: who replied, where, and the first words.
struct PushNotice: Decodable {
  let title: String
  let subtitle: String?
  let body: String
  let chat_id: String
}

/// The push envelope the Rust core seals (`crates/cli/src/push.rs`): `nonce(12) || ciphertext
/// || tag` of ChaCha20-Poly1305 under the push key, with `push` as associated data, carried
/// as unpadded base64url.
enum PushEnvelope {
  static func open(_ sealed: String, key: Data) -> PushNotice? {
    guard let envelope = base64url(sealed),
      let box = try? ChaChaPoly.SealedBox(combined: envelope),
      let plaintext = try? ChaChaPoly.open(box, using: SymmetricKey(data: key), authenticating: Data("push".utf8))
    else { return nil }
    return try? JSONDecoder().decode(PushNotice.self, from: plaintext)
  }

  static func base64url(_ text: String) -> Data? {
    var standard = text.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
    standard += String(repeating: "=", count: (4 - standard.count % 4) % 4)
    return Data(base64Encoded: standard)
  }
}
