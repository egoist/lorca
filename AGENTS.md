# Lorca

Read [ARCHITECTURE.md](./ARCHITECTURE.md) before writing code.

Identity is a local key pair. Paired Devices sync through an E2E relay. Provider credentials belong to the account: they sync to every paired Device as a blob encrypted with the account key. The AppKit app talks to the local CLI.

## Write the current system

Describe Lorca as it is: mechanisms, stack, and flows in the present tense.

When a constraint matters, name the thing that exists (AppKit, signed blobs, the encrypted `credentials` blob). Leave out ledgers of dropped accounts, old stack names, rejected services, and sections whose job is to list everything the project is not.
