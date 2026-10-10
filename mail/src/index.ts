// The mail Worker's entry. The runtime takes every export of this module for a handler, so the
// work is in `mail.ts` and only the handler is exported here.

import { receive, type Env } from "./mail";

export default {
  async email(message, env) {
    await receive(message, env);
  },
} satisfies ExportedHandler<Env>;
