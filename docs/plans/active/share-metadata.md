# Search and share metadata, icons and a share image

## Purpose

A link to the page shows a title, a description and an image when a person shares it on X, Facebook, Slack or a chat app.  Search engines get a description and one canonical URL.  The browser tab and the iPhone home screen show an icon.  Before this plan, the page had only a `<title>` and an empty icon (`<link rel="icon" href="data:,">`).

## Context and orientation

* `app/index.html` is Trunk's entry point.  Its head holds the tags.  Trunk does not fill values into the page, so each tag holds its value in full.
* `xtask publish` (`xtask/src/publish.rs`) puts each build in `v/<id>/` and copies the newest build's `index.html` to the root.  The root page's `<base href>` points into `v/<id>/`, so relative URLs resolve into the build's folder.
* The published site is `https://beachape.com/stemyla/` (README, "Publishing on GitHub Pages").
* Facts from the standards:
  * Open Graph needs `og:title`, `og:type`, `og:image` and `og:url`, and `og:image:alt` with an image ([ogp.me](https://ogp.me/)).
  * X takes the `og:` title, description and image when its own tags are missing.  `twitter:card` sets the large card ([X cards markup](https://developer.x.com/cards/markup)).
  * Meta asks for at least 1200 x 630 and the image's size in tags, so that the image shows on the first share ([Meta](https://developers.facebook.com/docs/sharing/webmasters/images/)).
  * Google takes a canonical link with an absolute URL as a strong signal across duplicate pages ([Google](https://developers.google.com/search/docs/crawling-indexing/consolidate-duplicate-urls)).  `v/<id>/index.html` duplicates the root page.
  * Google shows one favicon for each host, so a search result for a sub-folder shows the host's favicon ([Google](https://developers.google.com/search/docs/appearance/favicon-in-search)).
  * An ICO of 32 px, an SVG and a 180 px Apple touch icon cover current browsers; a manifest is only for an installable app ([Evil Martians](https://evilmartians.com/chronicles/how-to-favicon-in-2021-six-files-that-fit-most-needs)).

## Plan of work

* M1: `xtask brand` (`xtask/src/brand.rs`) draws `icon.svg`, `favicon.ico`, `apple-touch-icon.png` and `og.png` into `app/assets/`.  Acceptance: `cargo test -p xtask --test brand` passes.  `the_committed_brand_files_match_the_generator` fails when a file differs from what `brand` draws.
* M2: the head of `app/index.html` gets the description, the canonical link, the Open Graph tags, `twitter:card`, `theme-color` and the icon links, and copies the files into each build.  Acceptance: `the_head_tags_agree` passes, and `head_problems_finds_each_fault` shows that each fault gives a problem.
* M3: `xtask publish` requires the brand files in each build and copies the newest build's `og.png` to the site's root.  Acceptance: `cargo test -p xtask --test publish` passes, with `the_root_share_image_is_the_newest_build_s`.

## Validation and acceptance

* `cargo run -p xtask -- check --tier push` passes.
* `(cd app && trunk build --release)` puts the four files in `app/dist/`.
* After a publish, the link previews of X, Facebook and Slack show the image, the title and the description.  A person checks this, because it needs the published site.

## Idempotence and recovery

`xtask brand` writes the same files each run.  `brand --check` changes nothing.  To undo, revert the commits; the next publish removes the root `og.png`.

## Decisions for the maintainer

* Share image: Lanes, Unmix or Poster (mock-ups on 2026-11-17).  Recommended: Lanes.  Answer (2026-11-17): Unmix, with the footer "free, private, in your browser".
* Favicon: Bars, Lanes or Branch.  Recommended: Bars, the clearest at 16 px.  Answer (2026-11-17): Bars.
* Description.  Answer (2026-11-17): "Split any song into vocals, drums, bass, guitar, piano and the rest, in your browser with WebGPU. Free, private, no install: it all stems from your own GPU."
* How the images are made: a) `xtask brand` draws them in Rust, and a test compares the committed files; b) commit images made once.  Recommended: a).  Answer (2026-11-17): a).

## Progress

* [x] 2026-11-17: M1, `xtask brand` and its files.
* [x] 2026-11-17: M2, the head tags.
* [x] 2026-11-17: M3, the root share image in `publish`.
* [ ] Review rounds (decision 0024).

## Surprises and discoveries

* 2026-11-17: the demo song of `synth` has five stems, with no "other", and keeps one level for 30 s.  Its waveforms would give flat lanes.  So `brand` draws its waveforms from a seeded generator with a song's sections.
* 2026-11-17: the glyph rasteriser of `ab_glyph` uses only floor, ceil, square root and absolute value, and has no SIMD path.  So the test compares the pixels exactly, with no tolerance.

## Decision log

* 2026-11-17, maintainer: the choices in "Decisions for the maintainer"; decision 0033 records them.
* 2026-11-17, agent: the share image's waveforms come from a seeded generator, not from `synth` (see "Surprises and discoveries").
* 2026-11-17, agent: the tags name `https://beachape.com/stemyla/`.  A fork that publishes elsewhere changes them; the README says so.
* 2026-11-17, agent: the test compares PNGs by their pixels, not their bytes, so a new `png` encoder does not fail it.
* 2026-11-17, agent: `png` and `ab_glyph` go in `SETTLED_CRATES` (first versions 2015-05-26 and 2020-04-10 on crates.io).  `epaint_default_fonts` (first version 2024-09-26) is young, so it is pinned to the version in `Cargo.lock`.

## Outcomes and retrospective

To be filled in at the end.
