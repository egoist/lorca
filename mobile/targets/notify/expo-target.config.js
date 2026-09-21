/** @type {import('@bacons/apple-targets/app.plugin').ConfigFunction} */
module.exports = (config) => ({
  type: "notification-service",
  name: "LorcaNotify",
  displayName: config.name,
  bundleIdentifier: ".notify",
  deploymentTarget: "16.4",
  // The app leaves the push key in the group's keychain; the extension reads it there.
  entitlements: {
    "com.apple.security.application-groups": config.ios.entitlements["com.apple.security.application-groups"],
  },
});
