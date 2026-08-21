# Changelog workflow

`CHANGELOG.md` is generated from commits by [git-cliff](https://git-cliff.org/)
— do not edit it by hand, an edit is lost on the next rebuild. The parsing rules
live in `cliff.toml`.

```sh
./scripts/changelog.sh            # rebuild CHANGELOG.md
./scripts/changelog.sh unreleased # what piled up after the last tag
./scripts/changelog.sh v0.7.0     # notes for one release, to stdout
```

Only conventional commits make it in (`feat:`, `fix:`, `docs:`, …). `task:` and
`chore: version N` are skipped on purpose: the first are notes about keeping the
TODO list, the second are version bumps, and neither says anything to a reader
of the changelog.

A commit without such a prefix does not make it in at all — git-cliff puts the
whole commit text into the entry for those, and a squash merge would sprawl
across two pages.

## Release notes on GitHub

This file does **not** replace them. A GitHub release carries a human-written
explanation of *why* something changed; the changelog is a list of *what*
changed. The output of `changelog.sh <tag>` is a draft and a reminder of what
went into a release — but `gh release edit --notes` with it would overwrite what
was written by hand.

`--latest` in git-cliff always means the newest tag, never the one that was
asked for; that is why `changelog.sh <tag>` cuts the section out of the whole
changelog instead.
