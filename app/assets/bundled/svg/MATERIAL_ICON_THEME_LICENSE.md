The following icon files under `file_type/` and all icon files under `folder_type/` are
vendored from the [Material Icon Theme](https://github.com/PKief/vscode-material-icon-theme)
project (`icons/` directory), pinned to commit `2ad292ecbdb54cc4d4901fa5a72091e90bc612ba`
(2026-09-11):

`folder_type/`: all files (263 icons).

`file_type/`: all files (520 icons) except the following 21, which predate this vendoring
pass and were sourced separately: angular.svg, c.svg, cpp.svg, cython.svg, flash.svg, go.svg,
javascript.svg, json.svg, kotlin.svg, markdown.svg, mermaid.svg, npm.svg, perl.svg, php.svg,
python.svg, rust.svg, sql.svg, terraform.svg, typescript.svg, wasm.svg, zig.svg.

`app/src/code/icon_data.rs` is a mechanically-generated Rust port of the filename/extension
→ icon-name mapping from the same pinned commit's `src/core/icons/{fileIcons,folderIcons}.ts`
(the icon SVGs themselves are unmodified; only the mapping logic was reimplemented in Rust).

Material Icon Theme is distributed under the MIT License:

```
The MIT License (MIT)
Copyright (c) 2025 Material Extensions

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```
