# Contributing to Stitch

Thank you for your interest. Issues and pull requests are welcome.

## Before you start

- Read [README.md](README.md) and [docs/design.md](docs/design.md): the
  design document is the source of truth for decisions and open questions.
  For anything larger than a fix, open an issue first, so we can agree on
  the approach before you write it.
- [AGENTS.md](AGENTS.md) has the working rules for this repository, for
  people and coding agents alike: worktrees, the checks, dependencies.

## The checks

```sh
just quick          # while developing: fmt, clippy and tests of what changed
just check          # the full set: fmt, clippy -D warnings, all tests,
                    # licenses (cargo-deny), the third-party notices
just check-apple    # on macOS, when a change touches crates an Apple client links
```

A pull request passes `just check` (CI runs it). When you change
`Cargo.lock`, the web client's `bun.lock` or the llama.cpp submodule, run
`just notices` and commit the regenerated `notices/` files; a new Rust
dependency must have a license `deny.toml` allows.

## License and the Contributor License Agreement

Stitch is under the [Functional Source License, Version 1.1, Apache 2.0
Future License](LICENSE) (FSL-1.1-ALv2), from Fragcolor Pte. Ltd.

Contributions are accepted under a Contributor License Agreement:
[.github/CLA.md](.github/CLA.md). The CLA Assistant bot asks you to sign it
on your first pull request, with a comment; it covers all your later ones.

Why a CLA: the licensor has to be able to relicense the code. Each release
of Stitch becomes Apache-2.0 two years after it is made available, and
Fragcolor ships Stitch's node in its own clients (the official iOS and
visionOS apps), which are not open source. With the CLA you keep the
copyright to your work and grant Fragcolor the rights to do both. Without
it, we cannot merge your change.

## Security

Please do not open a public issue for a vulnerability. Report it privately
through GitHub's security advisories for this repository.
