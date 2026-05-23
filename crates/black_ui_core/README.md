# WarpUI

## Whirlwind tour

WarpUI contains many interlocking concepts. It's difficult to explain any one part of the system without reference to other parts. Because of this, this guide tries to provide an overview by exploring the relationships between the major concepts before providing a lot of detail on any one of them.

Rust's strict ownership rules are a challenge for user interfaces, where multi-directional dataflow is often critical. If every object has one and only one owner, how do we express things like event handlers?

## The global App object, entities, and handles

WarpUI solves this problem with the `App` object, which is the sole owner of all the views and models in the application. We collectively refer to views and models as **entities**. Entities can hold references to other entities via **handles**. A handle provides access to an entity in specific, limited circumstances. Take for example a multi-tabbed terminal. The application window is occupied by a `WorkspaceView`, and we want this workspace to contain multiple `TerminalView`s. Rather than holding the `TerminalView`s directly, the `Workspace` instead holds a vector of `ViewHandle<TerminalView>`.

```rust
struct WorkspaceView {
    sessions: Vec<ViewHandle<TerminalView>>,
}
```

On its own, a `ViewHandle` can't do much. Its existence prevents the referenced view from being discarded by the global `App` object, but it doesn't provide direct access to the referenced view. A handle is basically a glorified identifier. To convert a handle into an actual reference, you need a reference to an **app context** object, which will be provided by the global `App` object at specific points in time.

An example of one of those times could be the `render` method on `WorkspaceView`, which will be called by the framework whenever the workspace's on-screen representation is updated. One of the parameters to `render` is an `&AppContext`, which can be passed to the `as_ref` method on a `ViewHandle` to retrieve a reference to the underlying object.

Many details are elided in the code example below so we can focus on the topic at hand, but imagine we wanted to know the titles of all our terminal sessions so we could render them in tabs:

```rust
impl View for WorkspaceView {
    fn render<'a>(&self, ..., ctx: &AppContext) -> ... {
        let titles = self.sessions.iter().map(|handle| handle.as_ref(ctx).title()).collect::<Vec<String>>;
        ...
    }

    ...
}
```

After we return from the `render` method and lose access to the `&AppContext` parameter provided during its call, we no longer have access to the terminal views they reference.

Entities are of course entitled to own any state they like directly as well. Handles are only required when an entity needs to reference an entity. The reasons it would make sense to express a piece of application state as its own entity will become clearer as we sketch in more aspects of the system.

## Elements

The framework requires all views to implement the `View` trait, and a key method of this trait is `render`, which we showcased above. This method's job is to compute a visual description of the view based on its current state, and it is called whenever the view's state changes.

To describe the view's appearance, render returns an **element**. While a single view may exist for an arbitrary amount of time, changing its state as the user interacts with the application, an element is designed to exist for only a single frame. More precisely, elements returned by views that haven't changed are recycled across multiple frames, but conceptually you can think of an element as a throwaway object that is discarded and replaced whenever the view that returned it changes.

The framework ships with several elements that can be composed to perform common tasks such as drawing backgrounds and borders, adding padding, rendering label text, laying out elements horizontally and vertically, handling events, etc. The stock elements are loosely based on the Flutter framework. It's also straightforward to define your own custom elements, giving you detailed control over layout and the ability to imperatively paint pixels on scene via the hardware-accelerated `Scene` API.

## Actions

### Action handlers

### Action dispatch

## Views
