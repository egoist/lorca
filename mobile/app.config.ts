import type { ExpoConfig } from "expo/config";

export default (): ExpoConfig => {
  const development =
    process.env.LORCA_MOBILE_VARIANT === "development" ||
    process.env.EAS_BUILD_PROFILE === "development";
  const appName = development ? "Lorca Dev" : "Lorca";
  const appId = development ? "app.lorca.dev" : "app.lorca";
  const appGroup = development ? "group.app.lorca.dev" : "group.app.lorca";
  const icon = development ? "./assets/icon-dev.png" : "./assets/icon.png";
  const splash = development ? "./assets/splash-icon-dev.png" : "./assets/splash-icon.png";
  const favicon = development ? "./assets/favicon-dev.png" : "./assets/favicon.png";
  const adaptiveIconBackgroundColor = development ? "#ffbe00" : "#3424f5";
  const splashBackgroundColor = "#f7f7f8";
  const darkSplashBackgroundColor = "#1c1c1e";

  return {
    name: appName,
    slug: "lorca",
    version: "1.0.0",
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
      },
      buildNumber: "1",
      entitlements: {
        "com.apple.security.application-groups": [appGroup],
      },
      appleTeamId: "GJE9R5VE87",
    },
    android: {
      package: appId,
      adaptiveIcon: {
        backgroundColor: adaptiveIconBackgroundColor,
        foregroundImage: icon,
      },
      predictiveBackGestureEnabled: true,
      versionCode: 1,
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
