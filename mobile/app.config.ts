import type { ExpoConfig } from "expo/config";

// The phone app's version on both platforms. A release (`mobile-vX.Y.Z`) names it, and Android
// orders its builds by a versionCode made from it: 1.2.3 is 1002003.
const version = "1.0.0";
const [major, minor, patch] = version.split(".").map(Number);

export default (): ExpoConfig => {
  const development =
    process.env.LORCA_MOBILE_VARIANT === "development" ||
    process.env.EAS_BUILD_PROFILE === "development";
  const appName = development ? "Lorca Dev" : "Lorca";
  const appId = development ? "app.lorca.dev" : "app.lorca";
  const appGroup = development ? "group.app.lorca.dev" : "group.app.lorca";
  const icon = development ? "./assets/icon-dev.png" : "./assets/icon.png";
  const adaptiveIcon = development ? "./assets/adaptive-icon-dev.png" : "./assets/adaptive-icon.png";
  const splash = development ? "./assets/splash-icon-dev.png" : "./assets/splash-icon.png";
  const favicon = development ? "./assets/favicon-dev.png" : "./assets/favicon.png";
  const adaptiveIconBackgroundColor = development ? "#ffbe00" : "#3424f5";
  const splashBackgroundColor = "#f7f7f8";
  const darkSplashBackgroundColor = "#1c1c1e";

  return {
    name: appName,
    slug: "lorca",
    version,
    scheme: development ? "lorca-dev" : "lorca",
    orientation: "portrait",
    icon,
    userInterfaceStyle: "automatic",
    ios: {
      bundleIdentifier: appId,
      supportsTablet: true,
      infoPlist: {
        NSCameraUsageDescription: "Lorca scans a pairing QR code from another Device.",
        NSMicrophoneUsageDescription: "Lorca listens while you dictate a message.",
        NSSpeechRecognitionUsageDescription: "Lorca turns what you say into the message text.",
        NSPhotoLibraryUsageDescription: "Lorca attaches photos you pick to a message.",
        CFBundleAllowMixedLocalizations: true,
        // Export compliance, answered here so App Store Connect asks nothing per build. The core
        // encrypts with standard algorithms (X25519, Ed25519, ChaCha20-Poly1305, HKDF, SHA-2, and
        // TLS through rustls), not the system's, and Apple asks documentation for those only of an
        // app on the App Store in France: the French encryption declaration. With France in the
        // App Store availability, this becomes true beside ITSEncryptionExportComplianceCode, the
        // code Apple gives for the approved declaration. docs/releasing-mobile.md.
        ITSAppUsesNonExemptEncryption: false,
      },
      // scripts/release-ios.ts sets a fresh one for every upload; the notify extension takes the
      // same number through CURRENT_PROJECT_VERSION.
      buildNumber: process.env.LORCA_IOS_BUILD_NUMBER ?? "1",
      entitlements: {
        "com.apple.security.application-groups": [appGroup],
      },
      appleTeamId: "GJE9R5VE87",
    },
    android: {
      package: appId,
      // Firebase project lorca-a03db, with a client for app.lorca and app.lorca.dev: FCM tokens for pushes.
      googleServicesFile: "./google-services.json",
      // The launcher shows the middle 72 of the layer's 108 dp through its mask: the icon's art is
      // scaled into that square, its edges extended past it. The monochrome eyes are the themed icon.
      adaptiveIcon: {
        backgroundColor: adaptiveIconBackgroundColor,
        foregroundImage: adaptiveIcon,
        monochromeImage: "./assets/adaptive-icon-monochrome.png",
      },
      predictiveBackGestureEnabled: true,
      versionCode: major * 1_000_000 + minor * 1_000 + patch,
    },
    web: {
      favicon,
    },
    plugins: [
      "expo-router",
      "expo-secure-store",
      "expo-system-ui",
      [
        "expo-splash-screen",
        {
          backgroundColor: splashBackgroundColor,
          image: splash,
          imageWidth: 220,
          resizeMode: "contain",
          dark: {
            backgroundColor: darkSplashBackgroundColor,
            image: splash,
          },
        },
      ],
      [
        "expo-camera",
        {
          cameraPermission: "Lorca scans a pairing QR code from another Device.",
        },
      ],
      [
        "expo-image-picker",
        {
          photosPermission: "Lorca attaches photos you pick to a message.",
          cameraPermission: "Lorca attaches photos you take to a message.",
        },
      ],
      [
        "expo-speech-recognition",
        {
          microphonePermission: "Lorca listens while you dictate a message.",
          speechRecognitionPermission: "Lorca turns what you say into the message text.",
        },
      ],
      "expo-image",
      "expo-localization",
      "expo-notifications",
      "@bacons/apple-targets",
      "./plugins/with-scene-lifecycle",
      "./plugins/with-material-theme",
      "expo-web-browser",
    ],
    experiments: {
      typedRoutes: true,
    },
    locales: {
      en: "./locales/en.json",
      "zh-Hans": "./locales/zh-Hans.json",
    },
  };
};
