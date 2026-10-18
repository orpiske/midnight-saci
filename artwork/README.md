# Artwork

[`BRAND.md`](BRAND.md) is the visual identity guide the
[web console](../docs/web-ui.md) follows.

## Missing files

The guide names logo files that are not in the repository yet:

- `midnight-saci-logo.png` (full lockup)
- `midnight-saci-mark.svg` and `midnight-saci-mark.png` (round mark)
- `midnight-saci-icon.svg` and `midnight-saci-icon.png` (app icon and favicon)

Until they are added here, the console uses the typographic wordmark
(Bricolage Grotesque ExtraBold, "Midnight" in Moonlight, "Saci" in Cap Red)
and has no favicon. No substitute mark is drawn. When the files arrive, add
them here and use the mark in the navigation header and the icon as the
favicon (`web/index.html`), as section 2 of the guide describes.

## Functional color adjustments

A few guide colors are adjusted where they would miss WCAG 2.2 AA
contrast, for example white text on Cap Red buttons. The adjustments and
their contrast ratios are listed in
[the console's accessibility notes](../docs/web-ui.md#accessibility-and-the-brand)
and checked by `web/src/styles/contrast.test.ts`.
