# Tinybot

Read [ARCHITECTURE.md](./ARCHITECTURE.md) before writing code.

Identity is a local key pair. Paired Computers sync through an E2E relay ([Happy](https://happy.engineering/docs/security/)). Provider credentials live on the Computer a bot is assigned to. The AppKit app talks to the local CLI.

## Write the current system

Describe Tinybot as it is: mechanisms, stack, and flows in the present tense.

When a constraint matters, name the thing that exists (AppKit, signed blobs, credentials on the assigned Computer). Leave out ledgers of dropped accounts, old stack names, rejected services, and sections whose job is to list everything the project is not.
