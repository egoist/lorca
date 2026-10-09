// Links the system opens the app with. Open in Lorca on a shared bot's page is
// `lorca://t/<id>#<key>` (`lorca-dev://` for Lorca Dev): the key lives after the `#`, which a
// route would drop, so the link waits whole in the store until there is an account to read it
// with, and the root layout opens New Bot from Template on it. Every other link (`lorca://pair?…`)
// goes to its route.

import { useStore } from "../src/core/store";
import { templateAddress } from "../src/core/templates";

export function redirectSystemPath({ path }: { path: string; initial: boolean }): string | null {
  try {
    const link = /^lorca(?:-dev)?:\/\/t\//i.test(path) ? templateAddress(path) : undefined;
    if (!link) return path;
    useStore.setState({ pendingTemplateLink: link });
    return null;
  } catch {
    return path;
  }
}
