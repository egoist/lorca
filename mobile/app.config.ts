import type { ExpoConfig } from "expo/config";

export default (): ExpoConfig => {
  const development =
    process.env.LORCA_MOBILE_VARIANT === "development" ||
    process.env.EAS_BUILD_PROFILE === "development";
  const icon = development ? "./assets/icon-dev.png" : "./assets/icon.png";
  const splash = development ? "./assets/splash-icon-dev.png" : "./assets/splash-icon.png";
  const favicon = development ? "./assets/favicon-dev.png" : "./assets/favicon.png";
  const adaptiveIconBackgroundColor = development ? "#ffbe00" : "#3424f5";
  const splashBackgroundColor = "#f7f7f8";
  const darkSplashBackgroundColor = "#1c1c1e";

  return {
    name: "Lorca",
    slug: "lorca",
    version: "1.0.0",
    scheme: "lorca",
    orientation: "portrait",
    icon,
    userInterfaceStyle: "automatic",
    newArchEnabled: true,
    splash: {
      image: splash,
      resizeMode: "contain",
      backgroundColor: splashBackgroundColor,
    },
    ios: {
      bundleIdentifier: "app.lorca",
      supportsTablet: true,
      infoPlist: {
        NSCameraUsageDescription: "Lorca scans the pairing code your Mac shows.",
        NSMicrophoneUsageDescription: "Lorca listens while you dictate a message.",
        NSSpeechRecognitionUsageDescription: "Lorca turns what you say into the message text.",
        NSPhotoLibraryUsageDescription: "Lorca attaches photos you pick to a message.",
        CFBundleAllowMixedLocalizations: true,
      },
      buildNumber: "1",
      entitlements: {
        "com.apple.security.application-groups": ["group.app.lorca"],
      },
      appleTeamId: "GJE9R5VE87",
    },
    android: {
      package: "app.lorca",
      adaptiveIcon: {
        backgroundColor: adaptiveIconBackgroundColor,
        foregroundImage: icon,
      },
      edgeToEdgeEnabled: true,
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
          cameraPermission: "Lorca scans the pairing code your Mac shows.",
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
