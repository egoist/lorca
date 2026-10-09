// Android's app theme is Material 3 with the wallpaper's dynamic colors, so what Android draws for
// the app (alert dialogs, text selection handles and cursors, the overscroll glow) has Material 3's
// look and colors rather than AppCompat's. The template's theme is AppCompat; this swaps its parent
// and adds the Material Components library it comes from (expo-router already ships it).
const { withAndroidStyles, withAppBuildGradle } = require("expo/config-plugins");

const PARENT = "Theme.Material3.DynamicColors.DayNight.NoActionBar";
const LIBRARY = 'implementation("com.google.android.material:material:1.13.0")';

module.exports = function withMaterialTheme(config) {
  config = withAndroidStyles(config, (config) => {
    const theme = config.modResults.resources.style?.find((style) => style.$.name === "AppTheme");
    if (!theme) throw new Error("with-material-theme: the template has no AppTheme; update the plugin");
    theme.$.parent = PARENT;
    return config;
  });
  return withAppBuildGradle(config, (config) => {
    if (!config.modResults.contents.includes("com.google.android.material:material")) {
      config.modResults.contents = config.modResults.contents.replace(/dependencies\s*\{/, (match) => `${match}\n    ${LIBRARY}`);
    }
    return config;
  });
};
