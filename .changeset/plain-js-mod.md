---
"herdr-project-tiles": minor
---

The Herdr.app mod no longer needs a Rust binary: git state, web links, languages, folder availability and the metadata cache are done in plain JavaScript. The herdr TUI plugin is removed (it stays in the git history up to 2bdad21). Projects can live on a herdr-core machine (`"machine"` in projects.json).
