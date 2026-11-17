# 0034: publish puts the share image at the site's root

Status: proposed

The agent made this choice while it did `plans/active/share-metadata.md`.  The maintainer accepts or rejects it ("Decisions for the maintainer").

## Context

`og:image` needs an absolute URL (decision 0033).  A site that shows a link may keep the image's URL for days.  But each build is in its own folder, `v/<id>/`, and `publish` prunes old folders.  So a URL into a build folder stops working after a few publishes.

## Options

* a) `publish` copies the newest build's `og.png` to the site's root, as it does `index.html`.
* b) A URL into `v/<id>/`, which `publish` writes into the root page.
* c) A URL on another host.

## Decision

Option a).  `og:image` is `https://beachape.com/stemyla/og.png`.  `finish_site` in `xtask/src/publish.rs` copies the file, and `check_build` requires it in each build.

## Consequences

* The root `og.png` changes in place, as the root `index.html` does.  A site that shows a link may keep the old image for days, until it reads the page again.
* Option b) would give each build its own URL, but a shared link would lose its image when `publish` prunes that build.  Option c) adds a host to keep.
