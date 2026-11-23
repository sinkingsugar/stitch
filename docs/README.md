# Stitch documentation

Start with the [project README](../README.md). This index separates practical
user guides from architecture, implementation history and experimental work.

## Using Stitch

| Guide | Contents |
|---|---|
| [Setup](setup.md) | Install/update, services, pairing, providers, secrets, approvals, MCP, skills, places, memory and troubleshooting |
| [Feature guide](features.md) | Detailed shell, web, browser, app, memory and orchestration behavior; schedules, SSH and integration entry points |
| [Command overview](commands.md) | Public command families, subcommands and TUI commands, organized by task |
| [Full CLI help](cli.md) | Generated argument/default reference for the entire command tree |
| [Terminal UI](tui.md) | Slash commands, keys, message queue and navigation |
| [Push relay](../relay/README.md) | Phone push relay deployment and APNs configuration |
| [Events](events.md) | Events from outside programs: publishing, asks, subscriptions |
| [Rooms](rooms.md) | Conversations with several participants |

For a setup task, read the relevant setup section first; use command help for
exact flags in the installed build. The feature guide is the detailed
explanation, not another installation procedure.

## Extending and developing

| Document | Role |
|---|---|
| [CONTRIBUTING.md](../CONTRIBUTING.md) | How to contribute, the checks, the license and the Contributor License Agreement |
| [AGENTS.md](../AGENTS.md) | Repository workflow and verification requirements |
| [Design](design.md) | Source of truth for architecture, decisions, open questions and roadmap; dated sections include history |
| [Records](records.md) | Record format, graph, derived state, transcript and deletion semantics |
| [Browser control](browser-tool.md) | Browser architecture, permissions, original proposal and implementation progress |
| [Live apps specification](live-apps-spec.md) | Apps reading the node's live state |
| [App-owned threads example](../examples/app-threads/README.md) | Runnable example of app manifests, grants and thread APIs |
| [Self-improvement](self-improve.md) | Harness issues, fixer permissions, CI/landing, build health and staged rollouts; includes historical baseline |
| [Browser experiment](../tools/browser-b0/README.md) | Original browser-control feasibility experiment, not an install guide |
| [RustSEC advisories](advisories.md) | The advisories `cargo deny` reported and how each was resolved |

Skills expose instructions and, optionally, checked Otter tools. Start with
[setup §5a](setup.md#5a-tools-written-in-otter) and
[place settings/skills](setup.md#7-places-settings-and-skills-of-a-folder-or-checkout).
Otter is a [separate repository](https://github.com/sinkingsugar/otter).

## Experimental encrypted vault

The vault is separate from normal paired-node sync. It is not a production
backup service. Its client (`stitch-vault`, `stitch vault`) is here; the hosted
service is Fragcolor's. Read [the protocol](vault-protocol.md) (experimental:
encrypted objects, membership and recovery) before relying on it.

## License and notices

Stitch is under the [FSL-1.1-ALv2](../LICENSE) (each release becomes
Apache-2.0 two years after it is published). The third-party components it
includes and their licenses: [notices/](../notices/README.md). Outside
contributions come under a CLA: [CONTRIBUTING.md](../CONTRIBUTING.md).

When changing a command or feature, update its guide and command reference in
the same change. Regenerate full help with `tools/cli-reference.sh` against a
freshly built binary; do not hand-edit generated defaults. Keep roadmap and
historical text labeled rather than presenting planned work as available.
