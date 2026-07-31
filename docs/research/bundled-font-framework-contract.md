# Bundled font and framework asset contract

> Historical dependency research. The accepted production renderer bundles
> Inter under OFL-1.1, disables system font discovery, and has no CSS/JavaScript
> framework or Liquid/HTML stage.

Status: decision-ready research for [issue #9](https://github.com/CorVous/WireTerm/issues/9)  
Evidence checked: 2026-07-30

## Decision

WireTerm's MVP should bundle the **Classic font bundle only**, unchanged, and
must carry the notices described below. It should not bundle the TRMNL12,
TRMNL16, or TRMNL21 files, nor TRMNL's compiled framework CSS/JavaScript.
Those artifacts are downloadable but their published archives contain no
license grant for redistribution.

WireTerm should implement its own small, TRMNL-inspired CSS contract rather
than copying the TRMNL framework. This avoids making MVP distribution depend
on an undocumented license while preserving the desired Liquid + HTML/CSS
authoring model.

This is a product-engineering license inventory, not legal advice.

## Current upstream inventory

TRMNL's release page currently lists framework **3.1.8** (released
2026-07-23), alongside two font bundles released 2026-04-30. The page
describes versioned CSS and JavaScript downloads and recommends versioned
releases for production rather than `latest`.
([release index](https://trmnl.com/framework/releases),
[3.1.8 notes](https://trmnl.com/framework/releases/3.1.8.md))

| Artifact | Files / declared styles | Published terms | Classification for WireTerm |
| --- | --- | --- | --- |
| TRMNL12 | Regular and Bold; WOFF2, WOFF, TTF | Bundle README names Heavyweight Digital Type Foundry but supplies no license or copyright grant for this family. ([TRMNL bundle README](https://trmnl.com/fonts/bundles/trmnl/README.md)) | **Unverified — exclude from MVP** |
| TRMNL16 | Regular and Bold; WOFF2, WOFF, TTF | Same absence of a license grant. ([TRMNL bundle README](https://trmnl.com/fonts/bundles/trmnl/README.md)) | **Unverified — exclude from MVP** |
| TRMNL21 | Regular and Bold; WOFF2, WOFF, TTF | Same absence of a license grant. ([TRMNL bundle README](https://trmnl.com/fonts/bundles/trmnl/README.md)) | **Unverified — exclude from MVP** |
| Inter Variable | Upright and Italic variable TTF files | SIL Open Font License 1.1; copyright 2016–2020 The Inter Project Authors. The upstream Inter repository also publishes Inter under OFL-1.1. ([bundle README](https://trmnl.com/fonts/bundles/classic/README.md), [Inter license](https://github.com/rsms/inter/blob/master/LICENSE.txt)) | **Redistributable with OFL notice** |
| NicoPups | Regular/normal TTF | SIL Open Font License 1.1; copyright 2021 Emily Huo. ([Classic bundle README and full license](https://trmnl.com/fonts/bundles/classic/README.md)) | **Redistributable with OFL notice** |
| NicoClean | Regular/normal TTF | SIL Open Font License 1.1; copyright 2021 Emily Huo. ([Classic bundle README and full license](https://trmnl.com/fonts/bundles/classic/README.md)) | **Redistributable with OFL notice** |
| BlockKie | Regular/normal TTF | Creative Commons Attribution 3.0 Unported; copyright 2021 JoohnFonts. ([Classic bundle README](https://trmnl.com/fonts/bundles/classic/README.md), [CC BY 3.0 legal code](https://creativecommons.org/licenses/by/3.0/legalcode)) | **Redistributable with attribution** |
| Framework 3.1.8 CSS | `plugins.css`, `plugins.min.css`, and gzip variants | The release ZIP contains the binaries and release notes but no license or notice. The release page permits downloading to self-host, but does not state a redistribution license. ([release index](https://trmnl.com/framework/releases), [ZIP](https://trmnl.com/framework/trmnl-framework--3.1.8.zip)) | **Unverified — exclude from MVP** |
| Framework 3.1.8 JavaScript | `plugins.js`, `plugins.min.js`, and gzip variants | Same archive and absence of license terms. ([release index](https://trmnl.com/framework/releases), [ZIP](https://trmnl.com/framework/trmnl-framework--3.1.8.zip)) | **Unverified — exclude from MVP** |

The TRMNL font archive does reproduce the OFL for Inter, but it does **not**
say that the OFL applies to TRMNL12/16/21. A public download is not, by
itself, a redistribution license. The framework archive likewise has no
`LICENSE`, `COPYING`, or `NOTICE` file. WireTerm should treat both gaps as
unverified rather than infer permission.

## Required notices for the MVP bundle

Ship a third-party-notices file that is installed with WireTerm and reachable
from the application's About/help surface.

For NicoPups, NicoClean, and Inter, include:

- the applicable copyright line for each family;
- the complete SIL Open Font License 1.1 text; and
- the original family names, because WireTerm is distributing the files
  unmodified.

OFL 1.1 permits bundling and redistribution with software provided each copy
contains the copyright notice and license; the font files may not be sold by
themselves. Modified fonts remain under OFL and cannot use a Reserved Font
Name without permission when one is declared.
([OFL 1.1 text](https://openfontlicense.org/open-font-license-official-text/))

For BlockKie, include:

- `BlockKie — Copyright (c) 2021 JoohnFonts`;
- a statement that it is used under **CC BY 3.0**;
- a link or bundled copy of the CC BY 3.0 license; and
- an indication of changes. For the MVP's unchanged file, state
  `No changes were made`.

CC BY 3.0 permits sharing and adaptation, including commercially, subject to
appropriate credit, a license link, and an indication of changes.
([CC BY 3.0 deed](https://creativecommons.org/licenses/by/3.0/),
[legal code](https://creativecommons.org/licenses/by/3.0/legalcode))

## Exact MVP font payload

Bundle the five files from the Classic archive, without conversion or
subsetting:

- `NicoPups-Regular.ttf` — family NicoPups, normal;
- `NicoClean-Regular.ttf` — family NicoClean, normal;
- `BlockKie.ttf` — family BlockKie, normal;
- `Inter.ttf` — Inter Variable upright;
- `Inter-Italic.ttf` — Inter Variable italic.

This gives extensions the Classic low-density typography and a variable
high-density fallback while avoiding all unlicensed TRMNL-branded font files.
Do not copy the `@font-face` declarations from TRMNL's unlicensed compiled
CSS; define WireTerm-owned declarations that reference these local files.

## Pinning and upgrades

The font bundle URL is not versioned. Vendor the reviewed files in the
WireTerm source/distribution and record:

- upstream URL and advertised release date (`2026-04-30`);
- archive SHA-256
  `31ede14f07fe8d9fc0aa933453c3fb04da4d52e4977a955d113bf8318baef8a5`;
- each vendored file's SHA-256 in the dependency/notice manifest; and
- the exact license and copyright text reviewed with that payload.

Never fetch `latest` at runtime or during a release build. Font or framework
upgrades must be explicit dependency changes that re-check archive contents,
hashes, rendering snapshots, and license notices.

If TRMNL later publishes redistribution terms for its custom fonts or
framework, evaluate the exact licensed release as a new dependency; do not
retroactively treat today's archives as licensed.

## Extension-local fonts

Allow extension-local fonts in the MVP, with these boundaries:

- font files must live under the extension directory and be referenced by
  relative path; remote font URLs are rejected so rendering stays offline and
  deterministic;
- the extension manifest declares family, style/weight, file path, license
  identifier, and a notice/license file path;
- WireTerm loads them only for that extension's render and does not copy them
  into the application-wide bundle;
- the author is responsible for having embed/use rights, and WireTerm surfaces
  that responsibility in the authoring guide; and
- packaging or sharing an extension must include its declared notices.

This permits user-authored typography without making unreviewed fonts part of
WireTerm's own redistribution contract.

## Implementation acceptance criteria

1. The installer contains only the five Classic files listed above.
2. An automated distribution test rejects missing/mismatched font hashes and
   missing third-party notices.
3. No TRMNL framework CSS/JS or TRMNL12/16/21 file is present in source,
   installer, cache bootstrap, or runtime downloads.
4. Extension rendering resolves bundled and extension-local fonts without
   consulting the Windows font registry or the network.
5. The extension validator rejects absolute paths, paths escaping the
   extension root, remote font URLs, and missing license/notice declarations.
