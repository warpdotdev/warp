# Language Grammars
Example of how to add a new language grammar: https://github.com/warpdotdev/warp-internal/pull/11501/files

## TSLanguage
We need a [TSLanguage](https://tree-sitter.github.io/tree-sitter/using-parsers#the-basic-objects) object to parse a source code file. We use open-source libraries that provide functions that create these objects. These functions are usually written in C but we can use them in Rust crates.

## config.yaml
You can find this information on language specific documentation and style guides.

Another place to look is at Zed's config.toml files, e.g.: https://github.com/zed-industries/zed/blob/85bdd9329b550475aae34340e50abd4e79f2dd82/crates/languages/src/python/config.toml

**Note:** We don't use custom highlights.scm files - we use arborium's bundled highlighting queries instead.

## indents.scm
This controls how we know when a cursor should be indented relative to the previous line.

Other code editors need this information so we can base ours on other open-source code editors. We primarily need to support indent and outdent captures. More full-featured code editors will support more advanced features.

Some example sources to check include:
1. https://github.com/helix-editor/helix/blob/101a74bf6edbbfdf9b0628a0bdbbc307ebe10ff2/runtime/queries/python/indents.scm
1. https://github.com/zed-industries/zed/blob/85bdd9329b550475aae34340e50abd4e79f2dd82/crates/languages/src/python/indents.scm
