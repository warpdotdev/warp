pub mod button;
pub mod dialog;
pub mod keyboard_shortcut;
pub mod lightbox;
pub mod switch;
pub mod tooltip;

pub use keyboard_shortcut::KeyboardShortcut;
use warp_core::ui::appearance::Appearance;
use warpui::Element;

/// A reusable UI component that can be rendered with configurable parameters.
///
/// Components are designed to be long-lived and stored as fields in views rather than
/// created on every render. This is critical for components that maintain internal state
/// (such as mouse hover state via `MouseStateHandle`) - creating them fresh each render
/// will cause intra-frame state to be incorrect.
///
/// # Design Pattern
///
/// The component pattern separates:
/// - **Component struct**: Holds persistent state (mouse handles, tooltips, etc.)
/// - **Params struct**: Contains both required and optional rendering parameters
/// - **Options struct**: Contains only optional parameters with appearance-based defaults
///
/// This separation allows users to specify only what's necessary while getting sensible
/// defaults for everything else.
///
/// # Example
///
/// ```rust
/// use ui_components::{Component, Options, button};
/// use warp_core::ui::appearance::Appearance;
/// use warpui::prelude::*;
///
/// // Store component as a field in your view.
/// struct MyView {
///     my_button: button::Button,
/// }
///
/// impl MyView {
///     fn render_button(&self, appearance: &Appearance) -> Box<dyn warpui::Element> {
///         self.my_button.render(
///             appearance,
///             button::Params {
///                 // Required: specify what the button displays.
///                 content: button::Content::Label("Click me".into()),
///                 theme: &button::themes::Primary,
///                 // Optional: use defaults and override as needed.
///                 options: button::Options {
///                     disabled: false,
///                     ..Options::default(appearance)
///                 },
///             },
///         )
///     }
/// }
/// ```
///
/// # Implementing a New Component
///
/// ```rust
/// use ui_components::Component;
/// use warp_core::ui::appearance::Appearance;
/// use warpui::prelude::*;
///
/// // 1. Define the component struct with any persistent state.
/// #[derive(Default)]
/// pub struct MyComponent {
///     mouse_state: MouseStateHandle,
/// }
///
/// // 2. Define the params struct with required fields.
/// pub struct Params {
///     pub content: String,  // Required parameter.
///     pub options: Options,  // Optional parameters.
/// }
///
/// // 3. Define the options struct with optional fields.
/// pub struct Options {
///     pub disabled: bool,
///     pub size: f32,
/// }
///
/// // 4. Implement the traits.
/// impl ui_components::Params for Params {
///     type Options<'a> = Options;
/// }
///
/// impl ui_components::Options for Options {
///     fn default(appearance: &Appearance) -> Self {
///         Self {
///             disabled: false,
///             size: appearance.ui_font_size(),
///         }
///     }
/// }
///
/// impl Component for MyComponent {
///     type Params<'a> = Params;
///     
///     fn render<'a>(&self, appearance: &Appearance, params: Self::Params<'a>) -> Box<dyn Element> {
///         // Render implementation.
/// #       Empty::new().finish()
///     }
/// }
/// ```
pub trait Component: Default {
    /// The set of parameters that control rendering.
    ///
    /// This type should include both required parameters (fields that must always be
    /// specified, like button content) and an `options` field of type `Self::Params::Options`
    /// containing optional parameters.
    type Params<'a>: Params;

    /// Renders the component given the current application appearance and rendering parameters.
    ///
    /// This method is called during the render phase to produce the element tree for this
    /// component. The component can use its internal state (mouse handles, etc.) along with
    /// the provided parameters to determine how to render.
    fn render<'a>(&self, appearance: &Appearance, params: Self::Params<'a>) -> Box<dyn Element>;
}

/// The set of parameters that control rendering of a component.
///
/// This trait links a params struct to its corresponding options struct. The params struct
/// should contain:
/// - Required parameters as direct fields (e.g., button content, switch state)
/// - An `options: Self::Options` field for optional parameters
///
/// The lifetime parameter `'a` allows params and options to contain borrowed data, though
/// specific implementations may not use it.
///
/// # Example
///
/// ```rust
/// use ui_components::{Params, Options};
/// use warp_core::ui::appearance::Appearance;
///
/// pub struct MyParams {
///     pub content: String,   // Required.
///     pub options: MyOptions, // Optional.
/// }
///
/// pub struct MyOptions;
///
/// impl Options for MyOptions {
///     fn default(_: &Appearance) -> Self { Self }
/// }
///
/// impl Params for MyParams {
///     type Options<'a> = MyOptions;
/// }
/// ```
pub trait Params {
    /// The optional subset of parameters for this component.
    ///
    /// This type should contain only optional configuration that has sensible defaults.
    type Options<'a>: Options;
}

/// The optional subset of parameters that control rendering of a component.
///
/// Options provide appearance-based defaults for optional configuration, allowing users to
/// override only what they need. This trait requires implementing a `default` method that
/// computes appropriate defaults based on the current appearance (theme, font sizes, etc.).
///
/// # Design Philosophy
///
/// The distinction between required params and optional options allows for:
/// - **Compile-time safety**: Required parameters must be provided.
/// - **Convenience**: Optional parameters have sensible defaults.
/// - **Flexibility**: Defaults adapt to the current appearance/theme.
///
/// # Example
///
/// ```rust
/// use ui_components::{Options, MouseEventHandler};
/// use warp_core::ui::appearance::Appearance;
///
/// pub struct MyOptions {
///     pub disabled: bool,
///     pub font_size: f32,
///     pub on_click: Option<MouseEventHandler>,
/// }
///
/// impl Options for MyOptions {
///     fn default(appearance: &Appearance) -> Self {
///         Self {
///             disabled: false,
///             font_size: appearance.ui_font_size(),
///             on_click: None,
///         }
///     }
/// }
///
/// // Users can then use the struct update syntax to override specific options.
/// # fn example(appearance: &Appearance) {
/// let options = MyOptions {
///     disabled: true,
///     ..Options::default(&appearance)
/// };
/// # }
/// ```
pub trait Options {
    /// Computes default values for optional parameters based on the current appearance.
    ///
    /// This method should return sensible defaults that work well with the current theme,
    /// font sizes, and other appearance settings. The appearance parameter allows defaults
    /// to adapt to different visual contexts (light/dark theme, different font scales, etc.).
    fn default(appearance: &Appearance) -> Self;
}

/// A trait representing anything that can be rendered to an element tree given an appearance.
///
/// This trait provides a common interface for both UI components and custom rendering closures.
/// It's primarily used to allow components to accept flexible rendering parameters - either
/// sub-components or inline rendering logic.
///
/// # Implementations
///
/// There are two main implementations:
///
/// 1. **Component tuples**: `(&'a T, T::Params<'a>)` where `T: Component`
///    - Allows passing a component reference with its params.
///
/// 2. **Closures**: Any `FnOnce(&Appearance) -> Box<dyn Element>`
///    - Allows inline rendering logic.
///
/// # Example
///
/// ```rust
/// use ui_components::{Renderable, Options};
/// use warp_core::ui::appearance::Appearance;
/// use warpui::prelude::*;
///
/// pub struct SwitchOptions<'a> {
///     pub disabled: bool,
///     // Accept either a component or a rendering closure for the label.
///     pub label: Option<Box<dyn Renderable<'a>>>,
/// }
///
/// impl ui_components::Options for SwitchOptions<'_> {
///     fn default(_: &Appearance) -> Self {
///         Self { disabled: false, label: None }
///     }
/// }
///
/// // Usage with a closure.
/// # fn example(appearance: &Appearance) {
/// let options = SwitchOptions {
///     label: Some(Box::new(|appearance: &Appearance| {
///         Text::new("My Label", appearance.ui_font_family(), appearance.ui_font_size())
///             .finish()
///     })),
///     ..Options::default(&appearance)
/// };
/// # }
/// ```
pub trait Renderable<'a> {
    /// Renders this object into an element tree.
    fn render(self: Box<Self>, appearance: &Appearance) -> Box<dyn Element>;
}

/// An implementation of [`Renderable`] for any [`UiComponent`] and its parameters.
impl<'a, T: Component> Renderable<'a> for (&'a T, T::Params<'a>) {
    fn render(self: Box<Self>, appearance: &Appearance) -> Box<dyn Element> {
        self.0.render(appearance, self.1)
    }
}

/// An implementation of [`Renderable`] for any [`FnOnce`] that returns a [`Box<dyn Element>`].
impl<'a, T> Renderable<'a> for T
where
    T: FnOnce(&Appearance) -> Box<dyn Element>,
{
    fn render(self: Box<Self>, appearance: &Appearance) -> Box<dyn Element> {
        self(appearance)
    }
}

/// A function that handles mouse events.
pub type MouseEventHandler = Box<
    dyn FnMut(
        &mut warpui::EventContext,
        &warpui::AppContext,
        pathfinder_geometry::vector::Vector2F,
    ),
>;
