// The SF Symbol for a Device, the way the Mac sidebar picks it.
export function deviceSymbol(os: string, model: string): string {
  switch (os) {
    case "macos":
      if (model.includes("MacBook")) return "laptopcomputer";
      if (model.includes("Studio")) return "macstudio";
      if (model.includes("mini")) return "macmini";
      return "desktopcomputer";
    case "linux":
      return "server.rack";
    case "windows":
      return "pc";
    case "ios":
      return "iphone";
    case "ipados":
      return "ipad";
    default:
      return "smartphone";
  }
}
