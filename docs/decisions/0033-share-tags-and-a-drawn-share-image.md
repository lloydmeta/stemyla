# 0033: share tags, and icons and a share image that xtask draws

Status: accepted (2026-11-17, by the maintainer)

The maintainer chose the share image, the icon, the description and how the images are made.  `plans/active/share-metadata.md` records the choices ("Decisions for the maintainer").

## Context

A shared link to the page showed no image or description, and the tab showed no icon.  Open Graph and one X tag give the link preview.  A canonical link gives search engines one URL for the root page and its copies in `v/<id>/`.  `og:image` needs an absolute URL that stays the same across builds, but each build is in its own folder, and `publish` prunes old folders.

## Options

* Images: a) `xtask brand` draws them in Rust, and a test compares the committed files with what it draws.  b) Images made once and committed, with no generator.
* The share image's URL: a) `publish` copies the newest build's `og.png` to the site's root.  b) A URL into `v/<id>/`, which `publish` writes into the root page.  c) A URL on another host.

## Decision

Images a): `xtask brand` (`xtask/src/brand.rs`) draws the four files into `app/assets/`.  Trunk copies them into each build.  URL a): `publish` copies `og.png` to the root, as it does `index.html`.  `app/index.html` holds each tag's value in full.  The test `the_head_tags_agree` keeps a repeated value equal in each place, such as the title and `og:title`.

## Consequences

* A change of a stem colour in `app/src/lib.rs` fails `the_brand_colours_are_the_app_s` until `brand.rs` changes too.  Then run `cargo run -p xtask -- brand` and commit the files.
* `xtask` takes `png`, `ab_glyph` and `epaint_default_fonts`.  This adds no crate to the web app.  `png` and `epaint_default_fonts` already ship through eframe and egui, and `ab_glyph` does not ship.
* The tags name `https://beachape.com/stemyla/`.  A fork that publishes elsewhere changes them.
* A site that shows a link may keep the old image for days, until it reads the page again.
* Google shows the host's favicon in its results, not the page's, because the page is in a sub-folder.
