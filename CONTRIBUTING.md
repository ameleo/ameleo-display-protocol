# Contributing

Thank you for your interest. The interfaces are a contract between hosts and agents that
different people write, so please open an issue to discuss a change to an interface before
you start on it.

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md); report unacceptable behavior to
opensource@ameleo.fr.

## Changing an interface

Interfaces are versioned by name (`fr.ameleo.Agent.Mouse1`), and a published interface never
changes incompatibly. You may add a method, signal or property when existing peers can safely
ignore it. Any other change needs a new interface (`Mouse2`), and the old one stays until no
agent or host needs it anymore. Document each change in [CHANGELOG.md](CHANGELOG.md).

## Developer Certificate of Origin

Every commit must be signed off, certifying that you wrote it or have the right to submit it
under the project's license ([Developer Certificate of Origin](https://developercertificate.org/)):

    git commit -s

This adds a `Signed-off-by: Your Name <you@example.com>` line with your real name. A check on
every pull request refuses commits without it. No contributor license agreement is needed.

## License of contributions

The crate is under the MIT License or the Apache License 2.0, at the choice of whoever uses it.
By signing off a commit, you certify the Developer Certificate of Origin for it and license it
under those terms. Source files start with:

```rust
// SPDX-License-Identifier: MIT OR Apache-2.0
```

## Commits and pull requests

- Write code comments, documentation, commit messages and pull request descriptions in plain
  English, with full sentences.
- Keep each pull request to one topic. Describe why the change is needed, what it does, and how
  it was tested.
- Keep `cargo fmt`, `cargo clippy` and `cargo test` clean; continuous integration runs them on
  Linux and Windows.

Commit messages follow the usual Git conventions:

- A subject line of about 50 characters, in the imperative mood ("Add a camera control",
  "Fix the file list size"), without a trailing period.
- A blank line, then a body wrapped at 72 characters that explains why the change is needed and
  what it does.

```
Document the unit of PushFrame timestamps

The documentation did not say which unit timestamp_us uses, and a host
could reasonably send milliseconds. State that it is microseconds of
capture time on the client.

Signed-off-by: Your Name <you@example.com>
```

## AI-assisted contributions

AI coding assistants may be used, as in the Linux kernel and LLVM, on these conditions:

- A human reviews, understands and tests everything submitted, and answers for it in review.
- Substantial AI assistance is disclosed with an `Assisted-by:` trailer naming the assistant and
  model, and any tools it ran:

      Assisted-by: Claude:claude-opus-5-5
      Signed-off-by: Your Name <you@example.com>

- Only the human contributor signs off: an assistant cannot certify the Developer Certificate of
  Origin, and is not credited as a co-author.

Routine tools (editor completion of a few words, formatters, compilers) need no mention.

## Reporting security issues

Do not open a public issue for a vulnerability: see [SECURITY.md](SECURITY.md).
