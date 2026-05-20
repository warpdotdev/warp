use crate::actions::StandardAction;
use crate::AppContext;
use crate::{Action, Tracked};
use anyhow::anyhow;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    any::Any,
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};
use titlecase::titlecase;

mod context;
mod matcher;

use crate::platform::OperatingSystem;
pub use context::{macros, Context, ContextPredicate};
pub use matcher::{IsBindingValid, MatchResult, Matcher};

#[derive(Default)]
pub struct Keymap {
    fixed_bindings: Vec<FixedBinding>,
    editable_bindings: Vec<Tracked<EditableBinding>>,
    /// A mapping from binding name to indices in `editable_bindings` of bindings with
    /// that name, stored in the order they were registered.
    editable_bindings_by_name: HashMap<&'static str, Vec<usize>>,

    // We store a copy of the bindings, filtered down to only ones that are
    // triggered by a custom action.  This is done to optimize the lookups
    // of custom action bindings that are performed on macOS in response to
    // a `[WarpDelegate menuNeedsUpdate]` selector.
    fixed_custom_action_bindings: Vec<FixedBinding>,
    editable_custom_action_bindings: Vec<Tracked<EditableBinding>>,
}

// Custom actions should be identified by a unique integer called their tag.
pub type CustomTag = isize;

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub enum Trigger {
    Keystrokes(Vec<Keystroke>), // trigger when keys are pressed
    Standard(StandardAction),   // trigger when a StandardAction is dispatched
    Custom(CustomTag), // trigger when a Custom action (identified by its CustomTag) is dispatched
    Empty,             // empty trigger (cannot actually be matched)
}

impl Trigger {
    pub fn is_empty(&self) -> bool {
        matches!(self, Trigger::Empty)
    }
}

/// The context in which a binding description should be shown
#[derive(Debug, Clone, Copy)]
pub enum DescriptionContext {
    /// The default context (could be a command-palette or menu, depending on the app)
    Default,

    /// A custom, app-specific context specified by string
    Custom(&'static str),
}

/// Closure that can override a [`BindingDescription`] from live app state. See
/// [`BindingDescription::with_dynamic_override`].
pub type DynamicDescriptionResolver = Arc<dyn Fn(&AppContext) -> Option<String> + Send + Sync>;

#[derive(Default, Clone)]
/// A description of the binding.  Supports a single default context and
/// multiple custom contexts.  Custom contexts are effectively overrides.
///
/// May also carry a [`Self::with_dynamic_override`] resolver for bindings
/// whose label depends on live `&AppContext` state.
pub struct BindingDescription {
    // The default description.  If not overridden, it will be used in all
    // contexts.
    description: String,

    // A map of custom description contexts to custom descriptions
    custom: Option<HashMap<&'static str, String>>,

    // Optional dynamic override. The manual `PartialEq`/`Debug` impls below
    // intentionally ignore this field because equality is only consumed by
    // description deduplication that runs against already-materialized
    // `CommandBinding`s.
    dynamic_override: Option<DynamicDescriptionResolver>,
}

impl PartialEq for BindingDescription {
    fn eq(&self, other: &Self) -> bool {
        self.description == other.description && self.custom == other.custom
    }
}
impl Eq for BindingDescription {}

impl fmt::Debug for BindingDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BindingDescription")
            .field("description", &self.description)
            .field("custom", &self.custom)
            .field(
                "dynamic_override",
                &self.dynamic_override.as_ref().map(|_| "<dynamic>"),
            )
            .finish()
    }
}

impl BindingDescription {
    pub fn new<S: Into<String>>(description: S) -> Self {
        BindingDescription {
            description: titlecase(&description.into()),
            ..Default::default()
        }
    }

    pub fn new_preserve_case<S: Into<String>>(description: S) -> Self {
        BindingDescription {
            description: description.into(),
            ..Default::default()
        }
    }

    pub fn with_custom_description<S: Into<String>>(
        mut self,
        context: DescriptionContext,
        description: S,
    ) -> Self {
        if let DescriptionContext::Custom(key) = context {
            self.custom
                .get_or_insert_with(HashMap::new)
                .insert(key, description.into());
        } else {
            debug_assert!(false, "Expected custom description");
        }
        self
    }

    /// Attach a dynamic override for this description at materialization time.
    /// Returning `None` falls back to the static description for the requested
    /// context.
    ///
    /// The static value passed to [`Self::new`] is retained as the fallback
    /// for read paths that have no `&AppContext` (see [`Self::in_context`]).
    pub fn with_dynamic_override<F>(mut self, resolver: F) -> Self
    where
        F: Fn(&AppContext) -> Option<String> + Send + Sync + 'static,
    {
        self.dynamic_override = Some(Arc::new(resolver));
        self
    }

    /// True if this description has an attached dynamic override.
    pub fn has_dynamic_override(&self) -> bool {
        self.dynamic_override.is_some()
    }

    /// Returns the description for the given context, applying the dynamic
    /// override if one is attached and returns `Some`. Prefer this over
    /// [`Self::in_context`] anywhere `&AppContext` is in scope.
    pub fn resolve(&self, ctx: &AppContext, context: DescriptionContext) -> Cow<'_, str> {
        match &self.dynamic_override {
            Some(f) => match f(ctx) {
                Some(description) => Cow::Owned(titlecase(&description)),
                None => Cow::Borrowed(self.in_context(context)),
            },
            None => Cow::Borrowed(self.in_context(context)),
        }
    }

    /// Returns a static description with dynamic overrides resolved and removed.
    pub fn materialized(&self, ctx: &AppContext) -> Self {
        let mut description = BindingDescription::new_preserve_case(
            self.resolve(ctx, DescriptionContext::Default).into_owned(),
        );

        if let Some(custom) = &self.custom {
            description.custom = Some(
                custom
                    .keys()
                    .map(|&key| {
                        (
                            key,
                            self.resolve(ctx, DescriptionContext::Custom(key))
                                .into_owned(),
                        )
                    })
                    .collect(),
            );
        }

        description
    }

    /// Returns the static description for the given context. Does **not**
    /// invoke any attached dynamic override, so callers that have access
    /// to `&AppContext` should use [`Self::resolve`] instead. This method
    /// remains available for read paths that operate on
    /// already-materialized descriptions, or that genuinely cannot plumb
    /// a context through.
    pub fn in_context(&self, context: DescriptionContext) -> &str {
        match (context, &self.custom) {
            (DescriptionContext::Custom(key), Some(map)) => map
                .get(key)
                .map(|s| s.as_str())
                .unwrap_or_else(|| self.description.as_str()),
            _ => self.description.as_str(),
        }
    }
}

impl<S: Into<String>> From<S> for BindingDescription {
    fn from(description: S) -> Self {
        BindingDescription::new(description)
    }
}

/// A predicate that determines whether or not a binding is enabled. By default, all bindings are
/// enabled. Disabling a binding hides it completely - its context predicate never applies, it
/// should not be shown in keymap settings, and it cannot be triggered.
///
/// ## Enabled vs. Context Predicates
/// Context predicates configure whether or not a binding is available based on keymap contexts.
/// For example, many bindings are predicated on a particular view being focused. It's also common
/// to predicate bindings on view state, such as whether or not there's a text selection. Even if
/// its context predicate is false, the binding is still registered in the total set of bindings.
///
/// Enabled predicates dynamically decide if a binding is registered or not. They're similar to
/// conditionally calling [`warpui::app::AppContext::register_editable_bindings`], except
/// that the condition is re-evaluated at runtime. The main use for enabled predicates is to check
/// feature flags that might change post-initialization. If a feature is disabled, any bindings
/// related to it should be as well. Once the feature is enabled, the UI framework will start
/// checking the bindings' context predicates, and they can be shown in keymap settings.
pub type EnabledPredicate = fn() -> bool;

/// A lens into a binding, used to match keyboard events
/// to their appropriate actions.
#[derive(Copy, Clone, Debug)]
pub struct BindingLens<'a> {
    pub name: &'a str,
    pub trigger: &'a Trigger,
    pub action: &'a Arc<dyn Action>,
    context_predicate: &'a ContextPredicate,
    // BindingLens does not have an enabled predicate because we never construct a BindingLens for
    // disabled bindings.
    pub description: Option<&'a BindingDescription>,
    /// The original trigger for the binding. If `None`, the current trigger (set in `self.trigger`)
    /// and the original trigger are the same.
    pub original_trigger: Option<&'a Trigger>,
    pub group: Option<&'static str>,
    pub id: BindingId,
}

/// A unique identifier for a Binding within the application.
///
/// Used so that bindings can be uniquely identified even if data within them (such as their
/// trigger) changes.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct BindingId(pub usize);

static NEXT_BINDING_ID: AtomicUsize = AtomicUsize::new(0);

impl BindingId {
    /// Constructs a new globally-unique Binding ID.
    #[allow(clippy::new_without_default)]
    pub fn new() -> BindingId {
        let raw = NEXT_BINDING_ID.fetch_add(1, Ordering::Relaxed);
        BindingId(raw)
    }
}

/// This action can't be reconfigured with a custom key binding trigger
#[derive(Clone)]
pub struct FixedBinding {
    trigger: Trigger,
    action: Arc<dyn Action>,
    command_description: Option<BindingDescription>,
    context_predicate: ContextPredicate,
    enabled_predicate: Option<EnabledPredicate>,
    group: Option<&'static str>,
    /// A unique identifier that identifies this binding.
    id: BindingId,
}

/// An action which is explicitly registered with the key map
///
/// This action can have its key binding trigger overridden by setting a value
/// for `custom_trigger`.
#[derive(Clone)]
pub struct EditableBinding {
    name: &'static str,
    description: BindingDescription,
    action: Arc<dyn Action>,
    context_predicate: ContextPredicate,
    enabled_predicate: Option<EnabledPredicate>,
    trigger: Trigger,
    custom_trigger: Option<Trigger>,
    group: Option<&'static str>,
    /// A unique identifier that identifies this binding.
    id: BindingId,
}

/// A lens into an editable binding, allowing for the trigger to be updated where necessary
pub struct EditableBindingLens<'a> {
    pub name: &'static str,
    pub description: &'a BindingDescription,
    pub action: &'a Arc<dyn Action>,
    context: &'a ContextPredicate,
    enabled: Option<EnabledPredicate>,
    pub trigger: &'a Trigger,
    /// The original trigger, if a custom one is overriding it
    pub original_trigger: Option<&'a Trigger>,
    pub group: Option<&'static str>,
    pub id: BindingId,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Default, Serialize, Deserialize)]
pub struct Keystroke {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
    pub meta: bool,
    pub key: String,
}

/// In the user-visible settings schema (and in the TOML settings file), a
/// `Keystroke` is represented as a compact string like `"cmd-shift-a"`, not
/// as an object with per-modifier booleans. Serde continues to use the
/// default struct form for cloud sync and other in-memory consumers.
#[cfg(feature = "schema_gen")]
impl schemars::JsonSchema for Keystroke {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("Keystroke")
    }

    fn json_schema(gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        gen.subschema_for::<String>()
    }
}

#[cfg(feature = "settings_value")]
impl settings_value::SettingsValue for Keystroke {
    fn to_file_value(&self) -> serde_json::Value {
        serde_json::Value::String(self.normalized())
    }

    fn from_file_value(value: &serde_json::Value) -> Option<Self> {
        value.as_str().and_then(|s| Keystroke::parse(s).ok())
    }
}

pub trait ActionArg {
    fn boxed_clone(&self) -> Box<dyn Any>;
}

impl<T> ActionArg for T
where
    T: 'static + Any + Clone,
{
    fn boxed_clone(&self) -> Box<dyn Any> {
        Box::new(self.clone())
    }
}

impl Keymap {
    #[cfg(test)]
    pub fn new(fixed_bindings: Vec<FixedBinding>) -> Self {
        Self {
            fixed_bindings,
            ..Default::default()
        }
    }

    /// Returns the earliest-registered currently-enabled binding with the given name.
    pub fn get_binding_by_name(&self, name: &str) -> Option<BindingLens<'_>> {
        let indices = self.editable_bindings_by_name.get(name)?;
        indices.iter().find_map(|idx| {
            let binding = self.editable_bindings.get(*idx)?;
            let binding = binding.as_lens();
            binding.is_enabled().then_some(binding.as_binding())
        })
    }

    /// Add new fixed bindings to the keymap
    ///
    /// These bindings are internal and cannot be changed once they are added
    fn register_fixed_bindings<T: IntoIterator<Item = FixedBinding>>(&mut self, bindings: T) {
        let start_idx = self.fixed_bindings.len();
        self.fixed_bindings.extend(bindings);
        for binding in &self.fixed_bindings[start_idx..] {
            if matches!(binding.trigger(), Trigger::Custom(_)) {
                self.fixed_custom_action_bindings.push(binding.clone());
            }
        }
    }

    /// Add editable bindings to the keymap
    ///
    /// Editable Bindings have a name identifier which can be used to override their key bindings
    /// via the `set_custom_trigger` method.
    fn register_editable_bindings<A: IntoIterator<Item = EditableBinding>>(&mut self, actions: A) {
        let start_idx = self.editable_bindings.len();
        self.editable_bindings
            .extend(actions.into_iter().map(Tracked::new));
        for (idx, binding) in self.editable_bindings.iter().enumerate().skip(start_idx) {
            if matches!(binding.trigger, Trigger::Custom(_)) {
                self.editable_custom_action_bindings
                    .push(Tracked::new((*binding).clone()));
            }
            self.editable_bindings_by_name
                .entry(binding.name)
                .or_default()
                .push(idx);
        }
    }

    /// Updates the custom trigger for a given editable binding.
    fn update_custom_trigger(&mut self, name: &str, trigger: Option<Trigger>) {
        for binding in self
            .editable_custom_action_bindings
            .iter_mut()
            .filter(|b| b.name == name)
        {
            binding.custom_trigger = trigger.clone();
        }
        for binding in self.editable_bindings.iter_mut().filter(|b| b.name == name) {
            binding.custom_trigger = trigger.clone();
        }
    }

    /// Fetch an iterator of editable bindings
    ///
    /// The triggers for those actions will be overwritten by any custom triggers
    ///
    /// Items will be returned in the reverse order they were registered, the most recently
    /// registered editable binding will have the highest precedence
    fn editable_bindings(&self) -> impl Iterator<Item = EditableBindingLens<'_>> {
        self.editable_bindings
            .iter()
            .rev()
            .map(|binding| binding.as_lens())
            .filter(|binding| binding.is_enabled())
    }

    /// Fetch an iterator of `BindingLens` objects, with the editable key bindings
    /// modified by the custom bindings, where appropriate.
    ///
    /// Editable bindings will be returned first, followed by any fixed bindings in the reverse
    /// order they were added.
    fn bindings(&self) -> impl Iterator<Item = BindingLens<'_>> {
        self.editable_bindings()
            .map(|lens| lens.as_binding())
            .chain(
                self.fixed_bindings
                    .iter()
                    .rev()
                    .filter(|binding| binding.is_enabled())
                    .map(FixedBinding::as_lens),
            )
    }

    fn editable_custom_action_bindings(&self) -> impl Iterator<Item = EditableBindingLens<'_>> {
        self.editable_custom_action_bindings
            .iter()
            .rev()
            .map(|binding| binding.as_lens())
            .filter(|binding| binding.is_enabled())
    }

    pub(crate) fn custom_action_bindings(&self) -> impl Iterator<Item = BindingLens<'_>> {
        self.editable_custom_action_bindings()
            .map(|lens| lens.as_binding())
            .chain(
                self.fixed_custom_action_bindings
                    .iter()
                    .rev()
                    .filter(|binding| binding.is_enabled())
                    .map(FixedBinding::as_lens),
            )
    }
}

/// Struct that stores distinct keybindings depending on the platform the application is running on.
pub struct PerPlatformKeystroke {
    /// The binding that should be used on mac.
    pub mac: &'static str,
    /// The binding that should be used on linux and windows.
    pub linux_and_windows: &'static str,
}

impl FixedBinding {
    /// Constructs a new [`FixedBinding`] with separate bindings for mac and non-mac platforms.
    pub fn new_per_platform(
        keystroke: PerPlatformKeystroke,
        action: impl Action,
        context_predicate: ContextPredicate,
    ) -> Self {
        let keystroke = if OperatingSystem::get().is_mac() {
            keystroke.mac
        } else {
            keystroke.linux_and_windows
        };
        Self::new(keystroke, action, context_predicate)
    }

    /// Create a Key Binding for a Typed Action with the given keystrokes
    pub fn new<A>(
        keystrokes: impl AsRef<str>,
        action: A,
        context_predicate: ContextPredicate,
    ) -> Self
    where
        A: Action,
    {
        let keys = keystrokes
            .as_ref()
            .split_whitespace()
            .map(|key| Keystroke::parse(key).expect("Key Binding should be valid"))
            .collect();
        Self {
            trigger: Trigger::Keystrokes(keys),
            action: Arc::new(action),
            command_description: None,
            context_predicate,
            enabled_predicate: None,
            group: None,
            id: BindingId::new(),
        }
    }

    /// Create an empty binding for a typed action
    pub fn empty<D, A>(description: D, action: A, context_predicate: ContextPredicate) -> Self
    where
        A: Action,
        D: Into<BindingDescription>,
    {
        Self {
            trigger: Trigger::Empty,
            action: Arc::new(action),
            command_description: Some(description.into()),
            context_predicate,
            enabled_predicate: None,
            group: None,
            id: BindingId::new(),
        }
    }

    /// Create a Standard Action binding for a Typed action
    pub fn standard<A>(
        saction: StandardAction,
        action: A,
        context_predicate: ContextPredicate,
    ) -> Self
    where
        A: Action,
    {
        Self {
            trigger: Trigger::Standard(saction),
            action: Arc::new(action),
            command_description: None,
            context_predicate,
            enabled_predicate: None,
            group: None,
            id: BindingId::new(),
        }
    }

    /// Create a Custom Action (identified by its `CustomTag`) binding for a Typed Action
    pub fn custom<T, A, D>(
        caction: T,
        action: A,
        description: D,
        context_predicate: ContextPredicate,
    ) -> Self
    where
        T: Into<CustomTag>,
        A: Action,
        D: Into<BindingDescription>,
    {
        Self {
            trigger: Trigger::Custom(caction.into()),
            action: Arc::new(action),
            command_description: Some(description.into()),
            context_predicate,
            enabled_predicate: None,
            group: None,
            id: BindingId::new(),
        }
    }

    /// Sets the group for which this binding is a part of. This can be used to group bindings
    /// when reading all bindings from the [`Keymap`] (see [`Keymap::bindings`]).
    pub fn with_group(mut self, group: &'static str) -> Self {
        self.group = Some(group);
        self
    }

    /// Set a predicate for globally enabling/disabling this binding (by default, bindings are
    /// always enabled). See [`EnabledPredicate`] on when to use this instead of a context
    /// predicate.
    pub fn with_enabled(mut self, enabled: EnabledPredicate) -> Self {
        self.enabled_predicate = Some(enabled);
        self
    }

    pub fn trigger(&self) -> &Trigger {
        &self.trigger
    }

    pub fn action(&self) -> &dyn Action {
        &self.action
    }

    pub fn with_command_description<S: Into<BindingDescription>>(mut self, description: S) -> Self {
        self.command_description = Some(description.into());
        self
    }

    /// Determine if this binding is globally enabled. This must not be cached.
    ///
    /// See [`EnabledPredicate`] on why a binding might be disabled.
    fn is_enabled(&self) -> bool {
        self.enabled_predicate.is_none_or(|predicate| predicate())
    }

    /// Create a lens into this Binding's data
    fn as_lens(&self) -> BindingLens<'_> {
        BindingLens {
            name: Default::default(),
            trigger: &self.trigger,
            action: &self.action,
            context_predicate: &self.context_predicate,
            description: self.command_description.as_ref(),
            original_trigger: None,
            group: self.group,
            id: self.id,
        }
    }
}

impl EditableBinding {
    pub fn new<D, A>(name: &'static str, description: D, action: A) -> Self
    where
        D: Into<BindingDescription>,
        A: Action,
    {
        // Note: Explicitly not supporting registering legacy actions, as they will be removed
        // when the conversion to editable bindings is complete
        EditableBinding {
            name,
            description: description.into(),
            action: Arc::new(action),
            context_predicate: ContextPredicate::Just(true),
            enabled_predicate: None,
            group: None,
            trigger: Trigger::Empty,
            custom_trigger: None,
            id: BindingId::new(),
        }
    }

    pub fn with_context_predicate(mut self, context: ContextPredicate) -> Self {
        self.context_predicate = context;
        self
    }

    /// Set a predicate for globally enabling/disabling this binding (by default, bindings are
    /// always enabled). See [`EnabledPredicate`] on when to use this instead of a context
    /// predicate.
    pub fn with_enabled(mut self, enabled: EnabledPredicate) -> Self {
        self.enabled_predicate = Some(enabled);
        self
    }

    /// Sets the group for which this binding is a part of. This can be used to group bindings
    /// when reading all bindings from the [`Keymap`] (see [`Keymap::editable_bindings`]).
    pub fn with_group(mut self, group: &'static str) -> Self {
        self.group = Some(group);
        self
    }

    /// Sets the binding to that of `binding` if the current operating system is
    /// [`OperatingSystem::Mac`]. Noops otherwise.
    pub fn with_mac_key_binding<K>(self, binding: K) -> Self
    where
        K: AsRef<str>,
    {
        if OperatingSystem::get() == OperatingSystem::Mac {
            self.with_key_binding(binding)
        } else {
            self
        }
    }

    /// Sets the binding to that of `binding` if the current operating system is
    /// [`OperatingSystem::Linux`] or [`OperatingSystem::Windows`]. Noops otherwise.
    pub fn with_linux_or_windows_key_binding<K>(self, binding: K) -> Self
    where
        K: AsRef<str>,
    {
        if matches!(
            OperatingSystem::get(),
            OperatingSystem::Linux | OperatingSystem::Windows
        ) {
            self.with_key_binding(binding)
        } else {
            self
        }
    }

    pub fn with_key_binding<K>(mut self, binding: K) -> Self
    where
        K: AsRef<str>,
    {
        let keystrokes = binding
            .as_ref()
            .split_whitespace()
            .map(|key| Keystroke::parse(key).expect("Invalid keystroke"))
            .collect();
        self.trigger = Trigger::Keystrokes(keystrokes);
        self
    }

    pub fn with_standard_action(mut self, binding: StandardAction) -> Self {
        self.trigger = Trigger::Standard(binding);
        self
    }

    pub fn with_custom_action<C>(mut self, binding: C) -> Self
    where
        C: Into<CustomTag>,
    {
        self.trigger = Trigger::Custom(binding.into());
        self
    }

    fn as_lens(&self) -> EditableBindingLens<'_> {
        let (trigger, original_trigger) = if let Some(custom_trigger) = self.custom_trigger.as_ref()
        {
            (custom_trigger, Some(&self.trigger))
        } else {
            (&self.trigger, None)
        };
        EditableBindingLens {
            name: self.name,
            description: &self.description,
            action: &self.action,
            context: &self.context_predicate,
            enabled: self.enabled_predicate,
            trigger,
            original_trigger,
            group: self.group,
            id: self.id,
        }
    }
}

impl<'a> EditableBindingLens<'a> {
    /// Create a lens into the binding information for the underlying `EditableBinding`
    ///
    /// Will return `None` if there is no `trigger` since there is no associated key binding
    fn as_binding(&self) -> BindingLens<'a> {
        BindingLens {
            name: self.name,
            trigger: self.trigger,
            action: self.action,
            context_predicate: self.context,
            description: Some(self.description),
            original_trigger: self.original_trigger,
            group: self.group,
            id: self.id,
        }
    }

    /// Determine if this binding is globally enabled. This must not be cached.
    ///
    /// See [`EnabledPredicate`] on why a binding might be disabled.
    fn is_enabled(&self) -> bool {
        self.enabled.is_none_or(|predicate| predicate())
    }

    /// Determine if this action applies to the given context
    pub fn in_context(&self, context: &Context) -> bool {
        self.context.eval(context)
    }
}

lazy_static! {
    /// List of the valid special key names, used when parsing Keystrokes
    pub static ref VALID_SPECIAL_KEYS: HashSet<&'static str> = HashSet::from([
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "pageup",
        "pagedown",
        "backspace",
        "enter",
        "insert",
        "delete",
        "escape",
        "tab",
        "numpadenter",
        "f1",
        "f2",
        "f3",
        "f4",
        "f5",
        "f6",
        "f7",
        "f8",
        "f9",
        "f10",
        "f11",
        "f12",
        "f13",
        "f14",
        "f15",
        "f16",
        "f17",
        "f18",
        "f19",
        "f20",
    ]);
}

impl Keystroke {
    pub fn is_valid_key(key_name: &str) -> bool {
        key_name.chars().count() == 1 || Self::is_valid_special_key(key_name)
    }

    pub fn has_any_modifier(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.cmd || self.meta
    }

    pub fn is_unmodified(&self) -> bool {
        !self.has_any_modifier()
    }

    pub fn is_unmodified_key(&self, key: &str) -> bool {
        self.key == key && self.is_unmodified()
    }

    pub fn is_unmodified_enter(&self) -> bool {
        (self.key == "enter" || self.key == "numpadenter") && self.is_unmodified()
    }

    pub fn is_shift_tab(&self) -> bool {
        self.key == "tab" && self.shift && !self.ctrl && !self.alt && !self.cmd && !self.meta
    }

    /// Returns whether the `key` is the name of a valid special key. A key is considered "special"
    /// if it is the name of a nonprintable physical key on the keyboard, such as `backspace` or
    /// `enter`.
    pub fn is_valid_special_key(key_name: &str) -> bool {
        VALID_SPECIAL_KEYS.contains(key_name)
    }

    /// Attempts to create a new [`Keystroke`] from the given source string. The source string is
    /// assumed to be a string that contains a sequence of characters separated by `-`.
    ///
    /// ## Supported Modifiers
    /// The following modifiers are supported:
    /// * `cmd`: The command key on Mac.
    /// * `cmdorctrl`: Represents "cmd" on Mac and "ctrl" on Linux and Windows.
    /// * `ctrl`
    /// * `shift`
    /// * `alt`
    /// * `meta`
    ///
    ///
    /// ## Supported Keycodes
    /// The following key codes are supported:
    /// * `0-9`
    /// * `a-z`
    /// * `A-Z`
    /// * `f1`-`f20`
    /// * Various Punctuation: `)`, `!`, `@`, `#`, `$`, `%`, `^`, `&`, `*`, `(`, `:`, `;`, `:`, `+`,
    ///   `=`, `<`, `,`, `_`, `-`, `>`, `.`, `?`, `/`, `~`, `` ` ``, `{`, `]`, `[`, `|`,`\`, `}`.
    /// * `space`
    /// * `up`, `down`, `left`, `right`
    /// * `home` and `end`
    /// * `pageup` and `pagedown`
    /// * `backspace`
    /// * `enter`
    /// * `insert`
    /// * `delete`
    /// * `escape`
    /// * `tab`
    /// * `numpadenter`
    pub fn parse(source: impl AsRef<str>) -> anyhow::Result<Self> {
        let source = source.as_ref();
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut cmd = false;
        let mut meta = false;
        let mut key = None;

        let mut components = source.split('-').peekable();
        while let Some(component) = components.next() {
            match component {
                "ctrl" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                "cmd" => cmd = true,
                "meta" => meta = true,
                "cmdorctrl" => {
                    if OperatingSystem::get() == OperatingSystem::Mac {
                        cmd = true
                    } else {
                        ctrl = true
                    }
                }
                "space" => key = Some(String::from(" ")),
                _ => {
                    if let Some(component) = components.peek() {
                        if component.is_empty() && source.ends_with('-') {
                            key = Some(String::from("-"));
                            break;
                        } else {
                            return Err(anyhow!("Invalid keystroke `{}`", source));
                        }
                    } else if Self::is_valid_key(component) {
                        key = Some(component.into());
                    } else {
                        return Err(anyhow!("Unknown key `{}`", component));
                    }
                }
            }
        }

        // Make sure that we aren't accidentally registering a keybinding
        // with shift + lowercase (e.g. shift-r), which will never actually be
        // sent (since the OS sends ctrl-R in cases like this)
        if cfg!(debug_assertions) {
            let stroke = match &key {
                Some(key) if key.chars().count() == 1 => {
                    Some(key.chars().next().expect("Character should exist"))
                }
                _ => None,
            };
            match stroke {
                Some(stroke) if shift && stroke.is_lowercase() => {
                    panic!("Invalid keystroke - shift + letter should be uppercase: {source}")
                }
                Some(stroke) if !shift && stroke.is_uppercase() => panic!(
                    "Invalid keystroke - without shift, letter should be lowercase: {source}"
                ),
                _ => (),
            };
        }

        Ok(Keystroke {
            ctrl,
            alt,
            shift,
            cmd,
            meta,
            key: key.ok_or_else(|| anyhow!("Invalid keystroke: key is unset"))?,
        })
    }

    pub fn normalized(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("ctrl-");
        }
        if self.alt {
            s.push_str("alt-");
        }
        if self.shift {
            s.push_str("shift-");
        }
        if self.cmd {
            s.push_str("cmd-");
        }
        if self.meta {
            s.push_str("meta-");
        }
        s.push_str(match self.key.as_str() {
            " " => "space",
            k => k,
        });

        s
    }

    /// Returns the keybinding string using special characters to present ctrl/alt/shift/cmd keys.
    /// Can be used for displaying the key shortcuts in the UI. Use `normalized` when defining an
    /// actual trigger for the action.
    pub fn displayed(&self) -> String {
        let mut s = Vec::new();
        if self.ctrl {
            let character = if OperatingSystem::get().is_mac() {
                "⌃"
            } else {
                "Ctrl"
            };
            s.push(character.into());
        }
        if self.alt {
            let character = if OperatingSystem::get().is_mac() {
                "⌥"
            } else {
                "Alt"
            };
            s.push(character.into());
        }
        if self.shift {
            let character = if OperatingSystem::get().is_mac() {
                "⇧"
            } else {
                "Shift"
            };
            s.push(character.into());
        }
        if self.cmd {
            let character = if OperatingSystem::get().is_mac() {
                "⌘"
            } else {
                "Logo"
            };
            s.push(character.into());
        }
        if self.meta {
            s.push("Meta".into());
        }

        // Always treat the key as uppercase--this matches how operating systems and most
        // applications display keybindings.
        s.push(match self.key.as_str() {
            "up" => "↑".into(),
            "down" => "↓".into(),
            "left" => "←".into(),
            "right" => "→".into(),
            "\t" => "Tab".into(),
            " " => "Space".into(),
            "enter" => "⏎".into(),
            "backspace" => "⌫".into(),
            key => {
                // Capitalize the first letter of the key name
                key.chars()
                    .next()
                    .map(|c| c.to_ascii_uppercase())
                    .into_iter()
                    .chain(key.chars().skip(1))
                    .collect::<String>()
            }
        });

        if OperatingSystem::get().is_mac() {
            // On mac, we want to display compactly as "⌘I"
            s.join("")
        } else {
            // On windows and linux, we want to display "Ctrl Shift I" instead of "CtrlShiftI"
            s.join(" ")
        }
    }
}

#[cfg(test)]
#[path = "keymap_tests.rs"]
mod tests;
