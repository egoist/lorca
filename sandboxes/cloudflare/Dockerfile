# The machine a Runner's coding agents work in: git and GitHub's CLI to clone, push, and open
# pull requests, and the agents Lorca drives. Add what your projects build with.
FROM node:24-bookworm
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates git gh ripgrep python3 \
  && rm -rf /var/lib/apt/lists/*
# The postinstall scripts put the native binaries in place, so do not skip scripts.
RUN npm install --global @anthropic-ai/claude-code @openai/codex opencode-ai @earendil-works/pi-coding-agent
WORKDIR /workspace
CMD ["sleep", "infinity"]
