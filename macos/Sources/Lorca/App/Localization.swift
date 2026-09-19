import Foundation

/// The language the app's words are in. It is the one picked in Settings › General, else the
/// one macOS resolves for the bundle, and it can change while the app runs: `L()` reads the
/// table of the language in force, and `AppDelegate` builds the menu and the windows again
/// when `didChange` is posted. So `L()` is called when a view is built, never kept in a
/// `static let`.
enum AppLanguage {
    static let didChange = Notification.Name("AppLanguageDidChange")
    static let supported = ["en", "zh-Hans"]

    /// The pick ("en", "zh-Hans"); nil follows the system.
    private(set) static var chosen: String? = Preferences.appLanguage

    /// The language in force.
    static var code: String {
        chosen ?? Bundle.main.preferredLocalizations.first ?? "en"
    }

    static var isChinese: Bool { code.hasPrefix("zh") }

    /// For dates and numbers: the user's region and calendar in the app's language.
    private(set) static var locale = makeLocale()

    /// The `.lproj` the words come from.
    private(set) static var bundle = makeBundle()

    static func choose(_ code: String?) {
        guard code != chosen else { return }
        Preferences.appLanguage = code
        chosen = code
        bundle = makeBundle()
        locale = makeLocale()
        Format.languageChanged()
        NotificationCenter.default.post(name: didChange, object: nil)
    }

    private static func makeBundle() -> Bundle {
        Bundle.main.path(forResource: code, ofType: "lproj").flatMap(Bundle.init(path:)) ?? .main
    }

    private static func makeLocale() -> Locale {
        guard chosen != nil else { return .current }
        var components = Locale.Components(locale: .current)
        components.languageComponents = Locale.Language.Components(identifier: code)
        return Locale(components: components)
    }
}

/// The app's words in the user's language. The key is the English text, so a string with no
/// translation reads as written; the tables are `macos/Resources/<language>.lproj/Localizable.strings`,
/// which `scripts/app.ts` copies into the bundle.
func L(_ key: String) -> String {
    AppLanguage.bundle.localizedString(forKey: key, value: key, table: nil)
}

/// One English word that means two things: `L("Pairing", context: "device state")` looks up
/// `Pairing|device state`, and reads as "Pairing" where that has no translation.
func L(_ text: String, context: String) -> String {
    AppLanguage.bundle.localizedString(forKey: "\(text)|\(context)", value: text, table: nil)
}

/// A sentence with values in it: `L("%@ is working…", name)`. Arguments are `%@` for text and
/// `%d` for whole numbers; a translation reorders them with `%1$@`, `%2$@`.
func L(_ key: String, _ arguments: CVarArg...) -> String {
    String(format: L(key), locale: AppLanguage.locale, arguments: arguments)
}
