# Lorca

Read [ARCHITECTURE.md](./ARCHITECTURE.md) before writing code, then the subject docs it lists for the parts you change.

Identity is a local key pair. Paired Devices sync through an E2E relay. Provider credentials belong to the account: they sync to every paired Device as a blob encrypted with the account key. The AppKit app talks to the local CLI.

## Write the current system

Describe Lorca as it is: mechanisms, stack, and flows in the present tense.

When a constraint matters, name the thing that exists (AppKit, signed blobs, the encrypted `credentials` blob). Leave out ledgers of dropped accounts, old stack names, rejected services, and sections whose job is to list everything the project is not.

## One doc per subject

ARCHITECTURE.md is the overview every session reads in full, and its Subjects table lists the docs in `docs/architecture/`, one per subject. A change to a mechanism changes the doc that describes it, in the same change; plans, reviews, and reports are not docs.

`bun run check:docs` holds ARCHITECTURE.md to 16 KiB and each subject doc to 24 KiB, so either is read in one pass, and checks that every subject is listed and every relative link and `#heading` resolves. A doc that outgrows its budget is split by subject, with the new doc added to the table; facts are not cut to fit.
