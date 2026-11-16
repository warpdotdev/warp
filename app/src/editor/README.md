# Input Box

## 1. APIs of the Input Box

The input box is a child view named `EditorView` instantiated within `TerminalView`.

For the editor view to communicate with the terminal view, have the editor view send an action and register the action in the parent view.

For the terminal view to communicate with the editor view, the terminal view can directly access the editor view via a field in its struct. For it to input a newline, for example, it calls:

```
fn input_newline(&mut self, _: &(), ctx: &mut ViewContext<Self>) {
...
    self.input.update(ctx, |input, ctx| input.insert(&'\n'.into(), ctx));
...
```

## 2. Editor

### Buffer

The data structure that contains the text in the EditorView is the `Buffer`.

Its most commonly used API of the `Buffer` is `chars_at()`, which returns an iterator we can use to traverse characters from a certain point.

Under the hood, the buffer is a `SumTree<Fragment>`. Each `Fragment` has a `Text`. The `Text` consists of `text` which is an `Arc<str>` and `runs` which is a `SumTree<Run>`. A run describes how much space the fragment is taking.

Another useful API of `Buffer` is `line_len(row_number)` which gives you the length of a row number.

### Indexing into the Buffer

We use `Point` and `Offset` to index into the buffer. `Point` is a 2-dimensional location with `row` and `column` within the `Buffer`. `Offset` is a 1-dimensional `usize` within the `Buffer`. You can easily convert `Point` and `Anchor` to `Offset` via `to_offset()`. `Offset` is useful for traversing through the characters without having to worry about changing row and column numbers.

### DisplayPoints and DisplayMap

Another struct with `row` and `column` is the `DisplayPoint`. `DisplayPoint` is only relevant in `EditorView` and `DisplayMap`, but not the `Buffer`.
`DisplayPoint` describes the location in the `DisplayMap`. The `DisplayMap` describes how the points **appear**—the visual coordinate system. The `Buffer` has no concept of how it's being displayed and could in fact be displayed in multiple views.

The values of `DisplayPoint`s and `Point`s differ when code folding or softwrapping occurs. We can translate between the `DisplayPoint` and `Point` using the `DisplayMap`.

### Selections and Cursors

The input box supports the multiple selections we are used to in VSCode. So our `EditorView` has a `Vec<Selection>`.

A `Selection` has a `start` anchor and an `end` anchor to denote its start and end position.

Selections and cursors are closely intertwined—wherever there is a selection, there is a cursor. **A cursor on its own is just an empty selection where `start==end`.** As such, there is always at least one `Selection`, with the first selection being the cursor.

### Anchors

An `Anchor` is a bookmark into the text. It allows us to index for a relative position in the text even if its absolute position has changed. An `Anchor` can be converted to a `DisplayPoint` or a `Point`.

For example, imagine my cursor is between the characters 'l' and 'E' in 'PartialEq'. Say the absolute position is row 3 column 7:

```
Partial|Eq
```

Then, we add 10 characters:

```
Partial1234567890|Eq
```

The cursor's absolute position is now at row 3 column 17. To easily calculate this position, we can convert the `Anchor` to row 3 column 17.

#### `AnchorBias::Left`, `AnchorBias::Right`

The AnchorBias determines whether the cursor ends up on the right side or the left side of the inserted text. The above example is when AnchorBias is `Right`. `AnchorBias::Left` looks like this:

```
Partial|1234567890Eq
```

`Anchor`s are most useful when we introduce collaborative editing, and it is important for users to know where they are typing in even when another user inserts text in the line they are on.
