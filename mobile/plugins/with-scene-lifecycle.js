// iOS 27 asserts at launch unless the app adopts the UIScene life cycle. Expo ships the scene
// delegate (`ExpoAppSceneDelegate`, which creates the window and starts React Native into it);
// this plugin opts the generated project in: the scene manifest in Info.plist, and an
// AppDelegate that provides the factory and leaves the window to the scene.
const { withInfoPlist, withAppDelegate } = require("expo/config-plugins");

module.exports = function withSceneLifecycle(config) {
  config = withInfoPlist(config, (config) => {
    config.modResults.UIApplicationSceneManifest = {
      UIApplicationSupportsMultipleScenes: false,
      UISceneConfigurations: {
        UIWindowSceneSessionRoleApplication: [{ UISceneConfigurationName: "Default", UISceneDelegateClassName: "EXExpoAppSceneDelegate" }],
      },
    };
    return config;
  });
  return withAppDelegate(config, (config) => {
    let source = config.modResults.contents;
    if (!source.includes("ExpoReactNativeFactoryProvider")) {
      source = source.replace("class AppDelegate: ExpoAppDelegate {", "class AppDelegate: ExpoAppDelegate, ExpoReactNativeFactoryProvider {");
    }
    const started = /#if os\(iOS\) \|\| os\(tvOS\)\s*window = UIWindow\(frame: UIScreen\.main\.bounds\)\s*factory\.startReactNative\([\s\S]*?launchOptions: launchOptions\)\s*#endif\n/;
    if (!started.test(source) && source.includes("factory.startReactNative(")) {
      throw new Error("with-scene-lifecycle: the AppDelegate template changed; update the plugin");
    }
    config.modResults.contents = source.replace(started, "    // The scene delegate (ExpoAppSceneDelegate) creates the window and starts React Native.\n");
    return config;
  });
};
