# Basic parser

This parser is a type driven, recursive descent parser. This work in progress guide goes over a friendly overview of the workings of it with examples. In some places `Did you know?` notes are included, they are good to know tips because the parser relies on them.

# Steps
1. Lex. - [What happens when we start the parse](#what-happens-when-we-start-the-parse)
2. Lite Parse. - [Working with the tokens](#working-with-the-tokens)
3. Type driven full parse. - [Full parse](#)

## What happens when we start the parse?

Let's say we are interested in parsing the input `warp --disable-telemetry`. Command calls generally have the form `cmd <arg1> <arg2> <argN>` where `<arg>`s are positional parameters. Let's use the primary functions of the parser and inspect. The first step we do is calling the tokenizer (the function `lex` here):

```rust
let input = "warp --disable_telemetry";
let start_offset = 0;

let (tokens, _) = lex(input, start_offset);
println!("{:#?}", tokens);
```

The output is:

```rust
(
    [
        Token {
            contents: Baseline(
                "warp",
            ),
            span: Span {
                start: 0,
                end: 4,
            },
        },
        Token {
            contents: Space,
            span: Span {
                start: 4,
                end: 5,
            },
        },
        Token {
            contents: Baseline(
                "--disable-telemetry",
            ),
            span: Span {
                start: 5,
                end: 24,
            },
        },
    ],
    None,
)
```

In effect, we receive here the input source *tokenized*. The `start_offset` is used by the parser to calculate the spans of each bare word. Inspecting a little more we can notice the tokens with a `span` field on them. The `Span` [struct type](../meta.rs) has the fields `start` and `end` number that are helpful for knowing where something is.

#### Did you know? The `Span` struct type.

Let's use the span numbers from the output above, and use the `Span` struct associated `slice` function. This function takes a string as input and will return a slice of it using the `Span`'s `start` and `end` values. Let's create three `Span`s with the numbers from the output above and slice each from the input string fed to the lexer.

```rust
let input = "warp --disable-telemetry";

let word1 = Span::new(0,4);
let word2 = Span::new(4,5);
let word3 = Span::new(5,24);

assert_eq!(word1.slice(input), "warp");
assert_eq!(word2.slice(input), " ");
assert_eq!(word3.slice(input), "--disable-telemetry");
```

## Working with the tokens

The next step of the basic parser isn't all that different from lexing/parsing from traditional parsing. The task right now is to understand the boundaries of the tokens and get the general forms ready for a full parse. We are not interested here in doing anything more just yet since each of these tokens can be passed to commands that have no signature registered (*more on this later*). We call this step the `Lite` parse step.

A very simplistic view of the grammar rules below:

```
LiteRootNode    := LiteGroup
LiteGroup       := LitePipeline (';' LitePipeline)*
LitePipeline    := LiteCommand ('|' LiteCommand)*
LiteCommand     := argument+
// (*more grammar later*)
```

These are represented as structs that basic parser generates, they are:

```rust
pub struct LiteRootNode {
    pub groups: Vec<LiteGroup>,
}

pub struct LiteGroup {
    pub pipelines: Vec<LitePipeline>,
}

pub struct LitePipeline {
    pub commands: Vec<LiteCommand>,
}

pub struct LiteCommand {
    // this is important!
    pub parts: Vec<Spanned<String>>,
    pub post_whitespace: Option<Span>,
}
```

#### Did you know? The `Spanned<T>` generic struct.

The `LiteCommand` has a `parts` field that holds a vector of `Spanned<String>`. We mentioned about the `Span` type before. Here, we talk about a generic `Spanned<T>` that allows wrapping any `T` with a `Span` value along with it. Here is the type along with a few examples using it's helper functions.

```rust
pub struct Spanned<T> {
    pub span: Span,
    pub item: T,
}

let example = Spanned { item: String::from("warp"), span: Span::new(0,4) };
assert_eq!(example.item, "warp".to_string());
assert_eq!(example.span, Span::new(0,4));

let example = String::from("warp").spanned(Span::new(0,4));
assert_eq!(example.item, "warp".to_string());
assert_eq!(example.span, Span::new(0,4));

let example = "warp -p --disable-telemetry";

let full_span = Span::new(0, example.len());
let first_flag_span = Span::new(5,7);

assert_eq!(first_flag_span.slice(example), "-p");
assert_eq!(first_flag_span.until(full_span), Span::new(5,27));
assert_eq!(first_flag_span.until(full_span).slice(example), "-p --disable-telemetry");

```

This is useful because as soon as the `lite` parse step happens. We get as output everything we need with `spans` correctly calculated. Let's go ahead and do a lite parse (the function `parse_tokens`) to our original example using as input the tokens generated by the lexer when processing the input `warp --disable-telemetry`, like so:

```rust
let input = "warp --disable-telemetry";
let start_offset = 0;

let (tokens, _) = lex(input, start_offset);
let (lite_node, _) = parse_tokens(tokens);

let expected_word1 = String::from("warp").spanned(Span::new(0,4));
let expected_word2 = String::from("--disable-telemetry").spanned(Span::new(5,24));

assert_eq!(lite_node.groups[0].pipelines[0].commands[0].parts, vec![expected_word1, expected_word2]);
assert_eq!(lite_node.groups[0].pipelines[0].commands.len(), 1);

println!("{:#?}", lite_node);
```

We get a nice lite node:
```rust
LiteRootNode {
    groups: [
        LiteGroup {
            pipelines: [
                LitePipeline {
                    commands: [
                        LiteCommand {
                            parts: [
                                Spanned {
                                    span: Span {
                                        start: 0,
                                        end: 4,
                                    },
                                    item: "warp",
                                },
                                Spanned {
                                    span: Span {
                                        start: 5,
                                        end: 24,
                                    },
                                    item: "--disable-telemetry",
                                },
                            ],
                            post_whitespace: None,
                        },
                    ],
                },
            ],
        },
    ],
}
```

For more involved inputs (say commands separated by `|` and/or having `;`) the lite parser will effectively create the necessary `LitePipeline`s to express it, let's explore what happens if we lite parse the input `warp config-set --extension-path="/path/to/dir" ; echo $WARP_VAR"` (*two pipelines here due to the ; character*), like so:

```rust
let input = "warp config-set --extension-path=\"/path/to/dir\" ; echo $WARP_VAR";
let start_offset = 0;

let (tokens, _) = lex(input, start_offset);
let (lite_node, _) = parse_tokens(tokens);

println!("{:#?}", lite_node);
```

```rust
LiteRootNode {
    groups: [
        LiteGroup {
            pipelines: [
                LitePipeline {
                    commands: [
                        LiteCommand {
                            parts: [
                                Spanned {
                                    span: Span {
                                        start: 0,
                                        end: 4,
                                    },
                                    item: "warp",
                                },
                                Spanned {
                                    span: Span {
                                        start: 5,
                                        end: 15,
                                    },
                                    item: "config-set",
                                },
                                Spanned {
                                    span: Span {
                                        start: 16,
                                        end: 47,
                                    },
                                    item: "--extension-path=\"/path/to/dir\"",
                                },
                            ],
                            post_whitespace: Some(
                                Span {
                                    start: 47,
                                    end: 48,
                                },
                            ),
                        },
                    ],
                },
                LitePipeline {
                    commands: [
                        LiteCommand {
                            parts: [
                                Spanned {
                                    span: Span {
                                        start: 50,
                                        end: 54,
                                    },
                                    item: "echo",
                                },
                                Spanned {
                                    span: Span {
                                        start: 55,
                                        end: 64,
                                    },
                                    item: "$WARP_VAR",
                                },
                            ],
                            post_whitespace: None,
                        },
                    ],
                },
            ],
        },
    ],
}
 ```

 ## Type driven full parse

TODO
