use super::{
    BindingLens, Context, CustomTag, EditableBinding, EditableBindingLens, FixedBinding, Keymap,
    Keystroke, Trigger,
};
use crate::{actions::StandardAction, Action, EntityId};
use itertools::Either;
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
pub struct Matcher {
    pending: HashMap<EntityId, Pending>,
    keymap: Keymap,
    /// Default binding validator that should run on every binding (irrespective of the [`Context`]
    /// the binding was registered against).
    default_binding_validator: Option<BindingValidatorFn>,
    /// List of validators to be used during binding validation. Each binding validator validates
    /// all of the bindings that match the [`Context`] it is paired with.
    binding_validators: Vec<(Context, BindingValidatorFn)>,
    /// Function to convert bindings that have a [`CustomTag`] trigger to one that has a
    /// [`Keystroke`]-based trigger instead. If `None`, bindings are not converted.  
    custom_trigger_to_keystroke_fn: Option<Box<dyn Fn(CustomTag) -> Option<Keystroke> + 'static>>,
    /// Function to lookup the default keystroke for a given custom action. Used when converting
    /// custom actions to key events during keybinding editing.
    default_keystroke_trigger_for_custom_action:
        Option<Box<dyn Fn(CustomTag) -> Option<Keystroke> + 'static>>,
}

#[derive(Default)]
struct Pending {
    keystrokes: Vec<Keystroke>,
    context: Option<Context>,
}

type BindingValidatorFn = Box<dyn Fn(BindingLens) -> IsBindingValid>;

/// Enum indicating the results of validating a binding.
#[derive(Debug, PartialEq)]
pub enum IsBindingValid {
    /// The binding is valid.
    Yes,
    /// The binding is invalid.
    No,
}

pub enum MatchResult {
    None,
    Pending,
    Action(Arc<dyn Action>),
}

impl Matcher {
    pub fn new(keymap: Keymap) -> Self {
        Self {
            pending: HashMap::new(),
            keymap,
            default_binding_validator: None,
            binding_validators: vec![],
            custom_trigger_to_keystroke_fn: None,
            default_keystroke_trigger_for_custom_action: None,
        }
    }

    pub fn set_keymap(&mut self, keymap: Keymap) {
        self.pending.clear();
        self.keymap = keymap;
    }

    /// Helper function to that returns [`Trigger`] with any [`Trigger::Custom`]s replaced by a
    /// [`Trigger::Keystrokes`].
    fn convert_custom_trigger_to_keystroke_trigger(
        trigger: Trigger,
        custom_tag_to_keystroke: &dyn Fn(CustomTag) -> Option<Keystroke>,
    ) -> Trigger {
        let Trigger::Custom(custom_tag) = trigger else {
            return trigger;
        };

        let Some(new_keystroke) = custom_tag_to_keystroke(custom_tag) else {
            return trigger;
        };

        Trigger::Keystrokes(vec![new_keystroke])
    }

    pub fn register_fixed_bindings<T: IntoIterator<Item = FixedBinding>>(&mut self, bindings: T) {
        self.pending.clear();

        let bindings = match &self.custom_trigger_to_keystroke_fn {
            None => Either::Left(bindings),
            Some(custom_tag_to_keystroke) => {
                let bindings = bindings.into_iter().map(|mut fixed_binding| {
                    fixed_binding.trigger = Self::convert_custom_trigger_to_keystroke_trigger(
                        fixed_binding.trigger,
                        custom_tag_to_keystroke,
                    );
                    fixed_binding
                });
                Either::Right(bindings)
            }
        };
        self.keymap.register_fixed_bindings(bindings.into_iter());
    }

    /// Register new actions with the key matcher
    ///
    /// Editable Bindings have a name identifier which can be used to override their key bindings
    /// via the `set_custom_trigger` method.
    pub fn register_editable_bindings<A: IntoIterator<Item = EditableBinding>>(
        &mut self,
        actions: A,
    ) {
        self.pending.clear();

        let actions = match &self.custom_trigger_to_keystroke_fn {
            None => Either::Left(actions),
            Some(custom_tag_to_keystroke) => {
                let bindings = actions.into_iter().map(|mut editable_binding| {
                    editable_binding.trigger = Self::convert_custom_trigger_to_keystroke_trigger(
                        editable_binding.trigger,
                        custom_tag_to_keystroke,
                    );
                    editable_binding
                });
                Either::Right(bindings)
            }
        };
        self.keymap.register_editable_bindings(actions.into_iter());
    }

    /// Set a custom trigger for a given editable binding name.
    ///
    /// This will override the default trigger for that action.
    pub fn set_custom_trigger(&mut self, name: String, trigger: Trigger) {
        self.pending.clear();
        self.keymap
            .update_custom_trigger(name.as_str(), Some(trigger));
    }

    /// Remove any custom trigger associated with a given action.
    ///
    /// This will return the trigger to its default state.
    pub fn remove_custom_trigger<N>(&mut self, name: N)
    where
        N: AsRef<str>,
    {
        self.pending.clear();
        self.keymap.update_custom_trigger(name.as_ref(), None);
    }

    /// Registers a validator that validates every binding that matches the given view's default
    /// [`Context`].
    /// After the app is initialized, the provided `binding_validator` function is called for every
    /// binding that matches the View's default context. If the binding is invalid (indicated by
    /// [`IsBindingValid::No`]), the app will panic if `debug_assertions` are enabled.
    #[cfg(debug_assertions)]
    pub(crate) fn register_binding_validator<F: Fn(BindingLens) -> IsBindingValid + 'static>(
        &mut self,
        context: Context,
        binding_validator: F,
    ) {
        self.binding_validators
            .push((context, Box::new(binding_validator)));
    }

    /// Sets a default binding validator that runs on _every_ binding that is registered by the
    /// application.
    #[cfg(debug_assertions)]
    pub(crate) fn set_default_binding_validator<F: Fn(BindingLens) -> IsBindingValid + 'static>(
        &mut self,
        binding_validator: F,
    ) {
        self.default_binding_validator = Some(Box::new(binding_validator));
    }

    /// Runs through each registered binding validator, asserting that each matching binding is
    /// valid.
    #[cfg(debug_assertions)]
    pub(crate) fn validate_bindings(&mut self) {
        let mut all_failed_bindings = vec![];
        for (context, validator) in &self.binding_validators {
            for binding in self.bindings_for_context(context.clone()) {
                if let IsBindingValid::No = validator(binding) {
                    all_failed_bindings.push(binding);
                }
            }
        }

        if let Some(default_validator) = &self.default_binding_validator {
            for binding in self.get_bindings() {
                if let IsBindingValid::No = default_validator(binding) {
                    all_failed_bindings.push(binding);
                }
            }
        }

        if !all_failed_bindings.is_empty() {
            panic!("Bindings failed validation {all_failed_bindings:#?}");
        }
    }

    /// Overrides any registered binding that has a [`Trigger::Custom`] to one that is keystroke
    /// based ([`Trigger::Keystrokes`]) using the provided `custom_to_keystroke` fn.  
    pub(crate) fn convert_custom_triggers_to_keystroke_triggers(
        &mut self,
        custom_to_keystroke: impl Fn(CustomTag) -> Option<Keystroke> + 'static,
    ) {
        self.custom_trigger_to_keystroke_fn = Some(Box::new(custom_to_keystroke));
    }

    /// Registers a lookup function that returns the default keystroke for a given custom action.
    /// Used when converting custom actions to key events during keybinding editing.
    pub(crate) fn register_default_keystroke_triggers_for_custom_actions(
        &mut self,
        custom_to_keystroke: impl Fn(CustomTag) -> Option<Keystroke> + 'static,
    ) {
        self.default_keystroke_trigger_for_custom_action = Some(Box::new(custom_to_keystroke));
    }

    pub(crate) fn custom_action_bindings(&self) -> impl Iterator<Item = BindingLens<'_>> {
        self.keymap.custom_action_bindings()
    }

    /// Returns the first matching binding for the given custom action (not taking)
    /// into account the current context
    pub fn default_binding_for_custom_action(
        &self,
        custom_tag: CustomTag,
    ) -> Option<BindingLens<'_>> {
        self.keymap
            .bindings()
            // Filter out just the matching custom binding or action.
            // We look for matches against either the current or original trigger.
            .find(|binding| {
                matches!(
                    (binding.trigger, binding.original_trigger),
                    (Trigger::Custom(tag), _) | (_, Some(Trigger::Custom(tag))) if *tag == custom_tag
                )
            })
    }

    /// Returns any matching binding for the given custom tag and context
    pub fn binding_for_custom_action_in_context(
        &self,
        custom_tag: CustomTag,
        context: &Context,
    ) -> Option<BindingLens<'_>> {
        self.keymap
            .custom_action_bindings()
            // First filter out just the matching custom binding or action
            // We look for matches against either the current or original trigger.
            .filter(|binding| {
                matches!(
                    (binding.trigger, binding.original_trigger),
                    (Trigger::Custom(tag), _) | (_, Some(Trigger::Custom(tag))) if *tag == custom_tag
                )
            })
            // And then filter against the current context and return the first match
            .find(move |binding| binding.context_predicate.eval(context))
    }

    pub fn default_keystroke_trigger_for_custom_action(
        &self,
        custom_tag: CustomTag,
    ) -> Option<Keystroke> {
        self.default_keystroke_trigger_for_custom_action
            .as_ref()
            .and_then(|f| f(custom_tag))
    }

    pub fn get_binding_by_name(&self, name: &str) -> Option<BindingLens<'_>> {
        self.keymap.get_binding_by_name(name)
    }

    /// Returns an iterator of lenses to key bindings that apply to the given context.
    ///
    /// Key bindings are returned in precedence order, so the highest precedence key binding is
    /// returned first.
    pub fn bindings_for_context(&self, context: Context) -> impl Iterator<Item = BindingLens<'_>> {
        self.keymap
            .bindings()
            .filter(move |binding| binding.context_predicate.eval(&context))
    }

    /// Fetch an iterator of editable bindings
    ///
    /// The triggers for those actions will be overwritten by any custom triggers
    ///
    /// Items will be returned in the reverse order they were registered, the most recently
    /// registered editable binding will have the highest precedence
    pub fn editable_bindings(&self) -> impl Iterator<Item = EditableBindingLens<'_>> {
        self.keymap.editable_bindings()
    }

    /// Fetch an iterator of `BindingLens` objects, with the editable key bindings
    /// modified by the custom bindings, where appropriate.
    ///
    /// Editable bindings will be returned first, followed by any fixed bindings in the reverse
    /// order they were added.
    pub fn get_bindings(&self) -> impl Iterator<Item = BindingLens<'_>> {
        self.keymap.bindings()
    }

    pub fn push_keystroke(
        &mut self,
        keystroke: Keystroke,
        view_id: EntityId,
        ctx: &Context,
    ) -> MatchResult {
        let pending = self.pending.entry(view_id).or_default();

        if let Some(pending_ctx) = pending.context.as_ref() {
            if pending_ctx != ctx {
                pending.keystrokes.clear();
            }
        }

        pending.keystrokes.push(keystroke);

        let mut retain_pending = false;
        for binding in self.keymap.bindings() {
            if let Trigger::Keystrokes(keystrokes) = &binding.trigger {
                if keystrokes.starts_with(&pending.keystrokes)
                    && binding.context_predicate.eval(ctx)
                {
                    if keystrokes.len() == pending.keystrokes.len() {
                        self.pending.remove(&view_id);
                        return MatchResult::Action(binding.action.clone());
                    } else {
                        retain_pending = true;
                        pending.context = Some(ctx.clone());
                    }
                }
            }
        }

        if retain_pending {
            MatchResult::Pending
        } else {
            self.pending.remove(&view_id);
            MatchResult::None
        }
    }

    // Attempt to match with a StandardAction.
    // This returns None or Action, never Pending.
    pub fn match_standard(&self, action: StandardAction, ctx: &Context) -> MatchResult {
        for binding in self.keymap.bindings() {
            if let Trigger::Standard(triggeract) = binding.trigger {
                if *triggeract == action && binding.context_predicate.eval(ctx) {
                    return MatchResult::Action(binding.action.clone());
                }
            }
        }
        MatchResult::None
    }

    // Attempt to match with a CustomAction.
    // This returns None or Action, never Pending.
    pub fn match_custom(&self, action: CustomTag, ctx: &Context) -> MatchResult {
        for binding in self.keymap.bindings() {
            if let Trigger::Custom(tag) = binding.trigger {
                if *tag == action && binding.context_predicate.eval(ctx) {
                    return MatchResult::Action(binding.action.clone());
                }
            }
            if let Some(Trigger::Custom(tag)) = binding.original_trigger {
                if *tag == action && binding.context_predicate.eval(ctx) {
                    return MatchResult::Action(binding.action.clone());
                }
            }
        }
        MatchResult::None
    }
}

#[cfg(test)]
#[path = "matcher_tests.rs"]
mod tests;
