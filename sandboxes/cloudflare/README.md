# Lorca's Cloudflare Sandbox

A Worker you deploy to your own Cloudflare account so a Runner's coding agents run in [Cloudflare Containers](https://developers.cloudflare.com/containers/) instead of on the Runner. Each agent gets a sandbox of its own: a Durable Object and the Container it starts, with the repository cloned into `/workspace`. The Runner reaches the Worker with a token only you know, and passes each agent's stdin and output over a WebSocket. [Cloud sandboxes](../../docs/architecture/cloud-sandboxes.md) describes what Lorca does with it.

Containers need the Workers Paid plan. Building the image needs Docker running where you deploy.

## Deploy

```sh
cd sandboxes/cloudflare
npm install
printf '{"LORCA_TOKEN":"%s"}\n' "$(openssl rand -hex 32)" > .secrets.json
cat .secrets.json   # the token, for Lorca
npx wrangler deploy --secrets-file .secrets.json
rm .secrets.json
```

Wrangler prints the Worker's address, `https://lorca-sandbox.<you>.workers.dev`. Later deploys are `npx wrangler deploy`; `npx wrangler secret put LORCA_TOKEN` sets a new token.

## Set it up in Lorca

In Settings → Coding Agents, pick the Runner, then Cloudflare Sandbox, and give it the Worker's address, the token, and the variables your agents need there:

- `GITHUB_TOKEN` (or `GH_TOKEN`): git clones and pushes with it, and `gh` opens pull requests with it. A fine-grained token limited to the repositories you work on is enough.
- What each agent signs in with: `ANTHROPIC_API_KEY` or `CLAUDE_CODE_OAUTH_TOKEN` for Claude Code, `OPENAI_API_KEY` for Codex, and the keys OpenCode and pi use for your providers.

Check tells you whether the Worker answers with that token. Lorca keeps the token and the values on the Runner and replaces each with a placeholder wherever an agent prints it.

## The image

`Dockerfile` installs git, GitHub's CLI, Node.js, Python, and Claude Code, Codex, OpenCode, and pi. Lorca runs the first agent you turned on that the image has, so add what your projects build with, and any other agent, there. The Container may reach the internet; to limit it, see [Control network access](https://developers.cloudflare.com/sandbox/network/).

A Container sleeps two hours after the last thing its agent did, and its files go with it: a message to that agent after it slept cannot go on there, and the bot starts a new one for the job. Deleting the agent's chat, or Lorca forgetting the agent a month after it ended, ends its Container.
