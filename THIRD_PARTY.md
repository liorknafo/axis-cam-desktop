# Third-party components

AXIS application and helper source are MIT licensed. Rust dependency licenses remain those of their respective authors; versions are locked in Cargo.lock.

The distributed macOS STEP helper links dynamically to unmodified Homebrew Open CASCADE libraries and dependencies. Library copies are relocated with install_name_tool and ad-hoc signed. Their licenses and Homebrew version/source receipts are included in `Contents/Resources/ThirdParty`.

| Component | Version | License | Upstream source |
| --- | --- | --- | --- |
| Open CASCADE Technology | 7.9.3 | LGPL 2.1 with OCCT additional exception | [OCCT V7_9_3](https://github.com/Open-Cascade-SAS/OCCT/tree/V7_9_3) |
| oneTBB | 2023.1.0 | Apache 2.0 | [oneTBB](https://github.com/uxlfoundation/oneTBB) |
| FreeType | 2.14.3 | FreeType License | [FreeType VER-2-14-3](https://github.com/freetype/freetype/tree/VER-2-14-3) |
| libpng | 1.6.58 | libpng license | [libpng](https://github.com/pnggroup/libpng) |

Portions of this software are copyright © 2026 The FreeType Project (www.freetype.org). All rights reserved.

The CAD helper's source is `native/step_preview.cpp`. Rebuild it and the app with a C++17 compiler, Rust and an Open CASCADE installation: set OCCT_PREFIX and run the packaging script. The dynamically linked libraries can be replaced with compatible modified versions in `Contents/Frameworks`; re-sign the local bundle after replacing binaries. No license term of this application restricts modification or reverse engineering needed to debug changes to these libraries. No distribution signing credentials are required for local ad-hoc signing.

Format implementation references: [3MF core specification](https://github.com/3MFConsortium/spec_core), [3MF Production extension](https://github.com/3MFConsortium/spec_production), and [Open CASCADE](https://dev.opencascade.org/).

Stock surface meshing uses unmodified Fidget, fidget-core and fidget-mesh 0.4.3 under MPL-2.0. Their corresponding source is available at [Fidget v0.4.3](https://github.com/mkeeter/fidget/tree/v0.4.3), or the crates.io archives for those exact locked versions. Rebuild with Cargo to replace these libraries. The license and source notice are distributed in `licenses/fidget-MPL-2.0.txt` and `licenses/fidget-NOTICE.txt`, and copied to the app's `Contents/Resources/Licenses` directory.
